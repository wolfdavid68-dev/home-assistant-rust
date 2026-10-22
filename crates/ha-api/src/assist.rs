//! Assist (étape 54) : une phrase en français, tapée dans le tableau de bord
//! (« ferme le volet de la chambre »), comprise sur le serveur sans cloud.
//!
//! `POST /api/conversation/process` avec `{"text": "…"}`, comme Home
//! Assistant. La réponse : `{"response": {"response_type": …, "speech":
//! {"plain": {"speech": "C'est fait : fermer Volet chambre."}}}}`.
//!
//! Assist ne voit que les appareils que le compte a le droit de voir, et
//! chaque ordre passe par le même chemin qu'un toucher (`rest::execute`) :
//! droits, mot de passe redemandé, garde, valeurs vérifiées, journal d'audit.
//! Le texte de la phrase n'est pas écrit dans les logs.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use ha_assist::{Device, Understood};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ApiState;
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::rest::{Outcome, execute, parse_json};
use crate::roles;

/// Les appareils qu'Assist connaît.
const DOMAINS: [&str; 19] = [
    "light",
    "switch",
    "cover",
    "climate",
    "fan",
    "scene",
    "script",
    "lock",
    "valve",
    "humidifier",
    "water_heater",
    "input_boolean",
    "input_button",
    "button",
    "sensor",
    "binary_sensor",
    "person",
    "alarm_control_panel",
    // Les listes (courses, tâches) : « ajoute du pain à la liste de courses ».
    "todo",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessBody {
    text: String,
    /// Accepté pour ressembler à Home Assistant ; seul le français est compris.
    #[serde(default, rename = "language")]
    _language: Option<String>,
}

/// Les appareils visibles par ce compte, avec leur nom et leur pièce.
pub(crate) fn devices(state: &ApiState, auth: &Authenticated) -> Vec<Device> {
    state
        .core
        .states
        .all()
        .into_iter()
        .filter(|entity| DOMAINS.contains(&entity.entity_id().domain()))
        .filter(|entity| roles::can_see(&auth.user, entity.entity_id(), Some(entity)))
        .map(|entity| {
            let placed = state.areas.iter().find_map(|area| {
                area.entities
                    .iter()
                    .find(|member| member.entity_id == *entity.entity_id())
                    .map(|member| (area.name.clone(), member.name.clone()))
            });
            let name = entity
                .attributes()
                .get("friendly_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| placed.as_ref().and_then(|(_, name)| name.clone()))
                .unwrap_or_else(|| entity.entity_id().to_string());
            Device {
                state: entity,
                name,
                area: placed.map(|(area, _)| area),
            }
        })
        .collect()
}

fn reply(kind: &str, speech: &str) -> Json<Value> {
    Json(json!({ "response": response(kind, speech) }))
}

/// La réponse d'Assist, comme celle de Home Assistant.
pub(crate) fn response(kind: &str, speech: &str) -> Value {
    json!({
        "response_type": kind,
        "language": "fr",
        "speech": { "plain": { "speech": speech } },
    })
}

/// `POST /api/conversation/process`
pub(crate) async fn process(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let ProcessBody { text, .. } = parse_json(&body)?;
    let (kind, speech) = converse(&state, &auth, client, &text, false).await;
    Ok(reply(kind, &speech))
}

/// Comprend `text` et donne les ordres. Renvoie le type de réponse
/// (`action_done`, `query_answer`, `error`) et la phrase. `agent` : la
/// phrase vient d'un assistant IA (voir `rest::execute`).
pub(crate) async fn converse(
    state: &ApiState,
    auth: &Authenticated,
    client: SocketAddr,
    text: &str,
    agent: bool,
) -> (&'static str, String) {
    let devices = devices(state, auth);
    let understood = match ha_assist::understand(text, &devices) {
        Ok(understood) => understood,
        Err(not_understood) => {
            tracing::info!(
                "assist : phrase de {} non comprise ({not_understood:?})",
                auth.user.username
            );
            return ("error", not_understood.message());
        }
    };
    let orders = match understood {
        Understood::Answer(answer) => return ("query_answer", capitalized(&answer)),
        Understood::Orders(orders) => orders,
    };
    let mut done = Vec::new();
    let mut held = Vec::new();
    let mut failed = Vec::new();
    for order in orders {
        let mut summary = ha_notices::describe_order(
            &state.core.states,
            &order.domain,
            &order.service,
            &order.entity_id,
        );
        if let Some(amount) = ha_assist::amount(&order) {
            summary = format!("{summary} {amount}");
        }
        if order.service == "add_item"
            && let Some(item) = order.data.get("item").and_then(Value::as_str)
        {
            summary = format!("{item} ajouté à « {} »", order.name);
        }
        match execute(
            state,
            auth,
            client,
            &order.domain,
            &order.service,
            order.entity_id,
            order.data,
            agent,
        )
        .await
        {
            Ok(Outcome::Sent | Outcome::Done) => done.push(summary),
            Ok(Outcome::Held(reason)) => held.push(format!("{summary} ({reason})")),
            Err(ApiError::ConfirmationRequired) => failed.push(format!(
                "pour {summary}, redonne ton mot de passe : touche l'appareil dans le tableau de bord"
            )),
            Err(error) => failed.push(explain(error)),
        }
    }
    let mut parts = Vec::new();
    if !done.is_empty() {
        parts.push(format!("C'est fait : {}.", done.join(", ")));
    }
    if !held.is_empty() {
        parts.push(format!("En attente de validation : {}.", held.join(", ")));
    }
    if !failed.is_empty() {
        parts.push(format!("Pas fait : {}.", failed.join(" ; ")));
    }
    let kind = if failed.is_empty() {
        "action_done"
    } else {
        "error"
    };
    (kind, parts.join(" "))
}

/// Une majuscule au début.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Ce qu'une erreur veut dire, pour la personne (jamais le détail d'une
/// erreur interne).
pub(crate) fn explain(error: ApiError) -> String {
    match error {
        ApiError::BadRequest(message)
        | ApiError::Forbidden(message)
        | ApiError::DeviceRefused(message)
        | ApiError::Unavailable(message)
        | ApiError::DeviceTimeout(message) => message,
        ApiError::ConfirmationRequired => "redonne ton mot de passe".into(),
        ApiError::NotFound => "appareil introuvable".into(),
        ApiError::Unauthorized | ApiError::InvalidCredentials => "reconnecte-toi".into(),
        ApiError::TooManyRequests { .. } => "trop de demandes, réessaie dans un moment".into(),
        ApiError::Internal(_) => "erreur interne du serveur".into(),
    }
}
