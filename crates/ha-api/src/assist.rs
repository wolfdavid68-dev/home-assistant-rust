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
use crate::confirm;
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
                    .map(|member| {
                        (
                            Some(area.name.clone()),
                            member.name.clone(),
                            member.aliases.clone(),
                        )
                    })
            });
            // Les autres noms (étape 94) : seulement pour un appareil déjà
            // visible, donc jamais plus de droits que le nom.
            let (placed_area, label, aliases) = placed.unwrap_or_default();
            let name = entity
                .attributes()
                .get("friendly_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(label)
                .unwrap_or_else(|| entity.entity_id().to_string());
            // Sans pièce dans areas.yaml : celle que donne l'intégration
            // (la pièce SmartThings, étape 86).
            let area = placed_area.or_else(|| {
                entity
                    .attributes()
                    .get("piece")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
            Device {
                state: entity,
                name,
                aliases,
                area,
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
    // Abonné avant l'envoi : aucun changement d'état n'échappe (étape 95).
    let mut events = state.core.bus.subscribe();
    let mut watches = Vec::new();
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
        // Avant l'envoi : « bascule » se lit sur l'état d'avant.
        let watch = confirm::watch(
            &state.core.states,
            &order.domain,
            &order.service,
            &order.entity_id,
            &order.data,
            &order.name,
        );
        let entity_id = order.entity_id.clone();
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
            Ok(Outcome::Sent | Outcome::Done) => {
                done.push((entity_id, summary));
                watches.extend(watch);
            }
            Ok(Outcome::Held(reason)) => held.push(format!("{summary} ({reason})")),
            Err(ApiError::ConfirmationRequired) => failed.push(format!(
                "pour {summary}, redonne ton mot de passe : touche l'appareil dans le tableau de bord"
            )),
            Err(error) => failed.push(explain(error)),
        }
    }
    // Envoyé n'est pas fait : on attend un peu que les appareils obéissent.
    let unconfirmed = if watches.is_empty() {
        Vec::new()
    } else {
        confirm::wait(
            &mut events,
            &state.core.states,
            watches,
            confirm::CONFIRM_WITHIN,
        )
        .await
    };
    let unavailable = unconfirmed
        .iter()
        .any(|item| matches!(item, confirm::Unconfirmed::Unavailable(..)));
    let done: Vec<String> = done
        .into_iter()
        .filter(|(entity_id, _)| !unconfirmed.iter().any(|item| item.entity_id() == entity_id))
        .map(|(_, summary)| summary)
        .collect();
    let mut parts = Vec::new();
    if !done.is_empty() {
        parts.push(format!("C'est fait : {}.", done.join(", ")));
    }
    if let Some(sentence) = confirm::sentence(&unconfirmed) {
        parts.push(sentence);
    }
    if !held.is_empty() {
        parts.push(format!("En attente de validation : {}.", held.join(", ")));
    }
    if !failed.is_empty() {
        parts.push(format!("Pas fait : {}.", failed.join(" ; ")));
    }
    let kind = if failed.is_empty() && !unavailable {
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
