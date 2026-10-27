//! API WebSocket (`/api/websocket`), sur le modèle de celle de Home Assistant.
//!
//! Déroulé d'une connexion :
//!
//! 1. le serveur envoie `{"type": "auth_required"}` ;
//! 2. le client répond `{"type": "auth", "access_token": "hat_…"}` dans les 10 s ;
//! 3. le serveur répond `{"type": "auth_ok"}` (ou `auth_invalid` puis ferme) ;
//! 4. le client envoie des commandes `{"id": 1, "type": "…"}` :
//!    - `get_states` : tous les états ;
//!    - `subscribe_events` : reçoit ensuite chaque changement d'état ;
//!    - `config/entity_registry/get` (`entity_id`) et
//!      `config/entity_registry/get_entries` (`entity_ids`) : le nom, la
//!      pièce et les **autres noms** (`aliases`, étape 94) d'appareils
//!      visibles par ce compte ;
//!    - `ping` : répond `pong`.
//!
//! Le jeton passe dans le premier message, et non dans l'adresse
//! (`?token=…`) : les adresses se retrouvent souvent dans des logs.

use ha_auth::Permission;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use ha_auth::User;
use ha_core::{EntityId, Event};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use zeroize::Zeroizing;

use crate::ApiState;
use crate::json::StateJson;
use crate::roles;

/// Taille maximale d'un message reçu : nos commandes font quelques dizaines d'octets.
const MAX_MESSAGE_SIZE: usize = 64 * 1024;
/// Délai pour s'authentifier après l'ouverture.
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Fréquence à laquelle on revérifie le jeton d'une connexion ouverte : un
/// jeton révoqué (déconnexion, changement de mot de passe) coupe la connexion.
const RECHECK_INTERVAL: Duration = Duration::from_mins(1);
/// `config/entity_registry/get_entries` : appareils demandés, au plus.
const MAX_REGISTRY_IDS: usize = 500;

/// Pas de `Debug` : le jeton ne doit jamais être affiché.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthMessage {
    #[serde(rename = "type")]
    kind: String,
    access_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    id: u64,
    #[serde(rename = "type")]
    kind: String,
    /// Pour `config/entity_registry/get`.
    #[serde(default)]
    entity_id: Option<String>,
    /// Pour `config/entity_registry/get_entries`.
    #[serde(default)]
    entity_ids: Option<Vec<String>>,
}

/// `GET /api/websocket`
pub(crate) async fn handler(
    State(state): State<Arc<ApiState>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade
        .max_message_size(MAX_MESSAGE_SIZE)
        .max_frame_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| connection(socket, state))
}

async fn connection(mut socket: WebSocket, state: Arc<ApiState>) {
    if send(
        &mut socket,
        json!({"type": "auth_required", "ha_version": ha_core::VERSION}),
    )
    .await
    .is_err()
    {
        return;
    }

    let Some((user, token)) = authenticate(&mut socket, &state).await else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    if send(
        &mut socket,
        json!({"type": "auth_ok", "ha_version": ha_core::VERSION}),
    )
    .await
    .is_err()
    {
        return;
    }
    tracing::debug!("WebSocket ouvert pour {}", user.username);

    let username = user.username.clone();
    session(socket, &state, user, &token).await;
    tracing::debug!("WebSocket fermé pour {username}");
}

/// Attend le message d'authentification. `None` si absent, invalide ou trop tardif.
async fn authenticate(
    socket: &mut WebSocket,
    state: &ApiState,
) -> Option<(User, Zeroizing<String>)> {
    let message = tokio::time::timeout(AUTH_TIMEOUT, socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = message else {
        return None;
    };
    let invalid = |message: &str| json!({"type": "auth_invalid", "message": message});

    let Ok(auth) = serde_json::from_str::<AuthMessage>(&text) else {
        let _ = send(socket, invalid("message d'authentification attendu")).await;
        return None;
    };
    let token = Zeroizing::new(auth.access_token);
    if auth.kind != "auth" {
        let _ = send(socket, invalid("message d'authentification attendu")).await;
        return None;
    }
    match state.auth.authenticate(&token).await {
        Ok(Some(user)) => Some((user, token)),
        Ok(None) => {
            let _ = send(socket, invalid("jeton invalide ou expiré")).await;
            None
        }
        Err(error) => {
            tracing::error!("authentification WebSocket impossible : {error}");
            let _ = send(socket, invalid("erreur interne")).await;
            None
        }
    }
}

/// Boucle principale d'une connexion authentifiée.
async fn session(mut socket: WebSocket, state: &ApiState, mut user: User, token: &str) {
    // `Some((id, abonnement))` une fois `subscribe_events` reçu.
    let mut subscription: Option<(u64, broadcast::Receiver<Arc<Event>>)> = None;
    let mut recheck = tokio::time::interval(RECHECK_INTERVAL);
    recheck.reset(); // pas de vérification immédiate : on vient de le faire.
    let shutdown = state.shutdown_signal();

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    // Les ping/pong du protocole sont gérés automatiquement.
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    // Fermeture, message binaire (non prévu) ou erreur : on arrête.
                    _ => break,
                };
                let reply = match serde_json::from_str::<Command>(&text) {
                    Ok(command) => handle_command(&command, state, &user, &mut subscription),
                    Err(error) => json!({
                        "type": "result",
                        "success": false,
                        "error": {"code": "invalid_format", "message": error.to_string()},
                    })
                    .to_string(),
                };
                if send_text(&mut socket, reply).await.is_err() {
                    break;
                }
            }
            event = next_event(&mut subscription) => {
                let Some((id, event)) = event else { continue };
                // Un invité ne reçoit que ce qu'il a le droit de voir.
                if !visible(&user, &event) {
                    continue;
                }
                if send_text(&mut socket, event_message(id, &event)).await.is_err() {
                    break;
                }
            }
            _ = recheck.tick() => {
                // Jeton révoqué : on ferme. Rôle changé : il s'applique ici.
                match state.auth.authenticate(token).await {
                    Ok(Some(current)) => user = current,
                    _ => break,
                }
            }
            () = shutdown.cancelled() => break,
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

/// Cet événement peut-il être envoyé à ce compte ?
fn visible(user: &User, event: &Event) -> bool {
    match event {
        Event::StateChanged {
            entity_id,
            old_state,
            new_state,
        } => roles::can_see(
            user,
            entity_id,
            new_state.as_deref().or(old_state.as_deref()),
        ),
        // Commandes et événements nommés (qui a fait quoi) : pas pour un invité.
        Event::CallService { .. } | Event::Fired { .. } => user.can(Permission::AllDevices),
    }
}

fn handle_command(
    command: &Command,
    state: &ApiState,
    user: &User,
    subscription: &mut Option<(u64, broadcast::Receiver<Arc<Event>>)>,
) -> String {
    let id = command.id;
    let reply = match command.kind.as_str() {
        "ping" => json!({"id": id, "type": "pong"}),
        // La réponse la plus lourde : écrite directement depuis les états.
        "get_states" => {
            let all = state.core.states.all();
            let result = all
                .iter()
                .filter(|s| roles::can_see(user, s.entity_id(), Some(s)))
                .map(|s| StateJson::new(s))
                .collect();
            return to_text(
                id,
                &StatesReply {
                    id,
                    result,
                    success: true,
                    kind: "result",
                },
            );
        }
        "subscribe_events" if subscription.is_none() => {
            *subscription = Some((id, state.core.bus.subscribe()));
            json!({"id": id, "type": "result", "success": true, "result": null})
        }
        "subscribe_events" => error_result(id, "already_subscribed", "déjà abonné"),
        "config/entity_registry/get" => {
            match command
                .entity_id
                .as_deref()
                .and_then(|entity_id| registry_entry(state, user, entity_id))
            {
                Some(entry) => {
                    json!({"id": id, "type": "result", "success": true, "result": entry})
                }
                // Inconnu ou invisible pour ce compte : même réponse.
                None => error_result(id, "not_found", "appareil introuvable"),
            }
        }
        "config/entity_registry/get_entries" => {
            let Some(entity_ids) = &command.entity_ids else {
                return error_result(id, "invalid_format", "entity_ids attendu").to_string();
            };
            if entity_ids.len() > MAX_REGISTRY_IDS {
                return error_result(id, "invalid_format", "trop d'appareils demandés").to_string();
            }
            let entries: serde_json::Map<String, Value> = entity_ids
                .iter()
                .map(|entity_id| {
                    let entry = registry_entry(state, user, entity_id).unwrap_or(Value::Null);
                    (entity_id.clone(), entry)
                })
                .collect();
            json!({"id": id, "type": "result", "success": true, "result": entries})
        }
        _ => error_result(id, "unknown_command", "commande inconnue"),
    };
    reply.to_string()
}

/// Réponse à `get_states`. Les champs suivent l'ordre alphabétique, comme
/// dans un objet `serde_json::Value` (trié) : le texte reste le même qu'avant.
#[derive(Serialize)]
struct StatesReply<'a> {
    id: u64,
    result: Vec<StateJson<'a>>,
    success: bool,
    #[serde(rename = "type")]
    kind: &'static str,
}

/// `{"event": {"data": {…}, "event_type": "state_changed"}, "id": …, "type": "event"}`,
/// champs dans l'ordre alphabétique (voir [`StatesReply`]).
#[derive(Serialize)]
struct StateChangedMessage<'a> {
    event: StateChangedEvent<'a>,
    id: u64,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct StateChangedEvent<'a> {
    data: StateChangedData<'a>,
    event_type: &'static str,
}

#[derive(Serialize)]
struct StateChangedData<'a> {
    entity_id: &'a str,
    new_state: Option<StateJson<'a>>,
    old_state: Option<StateJson<'a>>,
}

/// Écrit un message en JSON. Cela n'échoue qu'avec des clés d'objet qui ne
/// sont pas du texte, ce que nos types n'ont pas ; le client reçoit alors une
/// erreur plutôt qu'un message vide.
fn to_text(id: u64, message: &impl Serialize) -> String {
    serde_json::to_string(message).unwrap_or_else(|error| {
        tracing::error!("message WebSocket impossible à écrire : {error}");
        error_result(id, "internal_error", "erreur interne").to_string()
    })
}

/// L'entrée « registre » d'un appareil, comme Home Assistant la donne :
/// son nom et sa pièce d'`areas.yaml`, et ses autres noms (étape 94).
/// `None` si l'appareil est inconnu ou invisible pour ce compte : un invité
/// n'apprend rien de plus par ses autres noms.
fn registry_entry(state: &ApiState, user: &User, entity_id: &str) -> Option<Value> {
    let entity_id: EntityId = entity_id.parse().ok()?;
    let current = state.core.states.get(&entity_id);
    let placed = state.areas.iter().find_map(|area| {
        area.entities
            .iter()
            .find(|member| member.entity_id == entity_id)
            .map(|member| (area, member))
    });
    if current.is_none() && placed.is_none() {
        return None;
    }
    if !roles::can_see(user, &entity_id, current.as_deref()) {
        return None;
    }
    let attribute = |key: &str| {
        current
            .as_ref()
            .and_then(|s| s.attributes().get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let area = placed
        .map(|(area, _)| area.name.clone())
        .or_else(|| attribute("piece"));
    Some(json!({
        "entity_id": entity_id.as_str(),
        "name": placed.and_then(|(_, member)| member.name.clone()),
        "original_name": attribute("friendly_name"),
        "aliases": placed.map(|(_, member)| member.aliases.clone()).unwrap_or_default(),
        "area_id": area.as_deref().map(ha_core::slugify),
        "platform": state.core.states.owner_of(&entity_id),
    }))
}

fn error_result(id: u64, code: &str, message: &str) -> Value {
    json!({
        "id": id,
        "type": "result",
        "success": false,
        "error": {"code": code, "message": message},
    })
}

/// Prochain événement de l'abonnement, ou attente infinie s'il n'y en a pas.
///
/// Concept Rust : `std::future::pending()` est une attente qui ne se termine
/// jamais. Dans un `select!`, cette branche est alors simplement ignorée.
async fn next_event(
    subscription: &mut Option<(u64, broadcast::Receiver<Arc<Event>>)>,
) -> Option<(u64, Arc<Event>)> {
    let Some((id, receiver)) = subscription else {
        return std::future::pending().await;
    };
    match receiver.recv().await {
        Ok(event) => Some((*id, event)),
        Err(RecvError::Lagged(missed)) => {
            tracing::warn!("client WebSocket trop lent : {missed} événements perdus");
            None
        }
        Err(RecvError::Closed) => {
            *subscription = None;
            None
        }
    }
}

fn event_message(id: u64, event: &Event) -> String {
    let message = match event {
        // Le plus fréquent : écrit directement depuis les états partagés.
        Event::StateChanged {
            entity_id,
            old_state,
            new_state,
        } => {
            return to_text(
                id,
                &StateChangedMessage {
                    event: StateChangedEvent {
                        data: StateChangedData {
                            entity_id: entity_id.as_str(),
                            new_state: new_state.as_deref().map(StateJson::new),
                            old_state: old_state.as_deref().map(StateJson::new),
                        },
                        event_type: "state_changed",
                    },
                    id,
                    kind: "event",
                },
            );
        }
        Event::CallService {
            domain,
            service,
            entity_id,
            data,
            ..
        } => json!({
            "id": id,
            "type": "event",
            "event": {
                "event_type": "call_service",
                "data": {
                    "domain": domain,
                    "service": service,
                    "service_data": {"entity_id": entity_id.as_str(), "data": data},
                },
            },
        }),
        Event::Fired { event_type, data } => json!({
            "id": id,
            "type": "event",
            "event": {"event_type": event_type, "data": data},
        }),
    };
    message.to_string()
}

async fn send(socket: &mut WebSocket, value: Value) -> Result<(), axum::Error> {
    send_text(socket, value.to_string()).await
}

async fn send_text(socket: &mut WebSocket, text: String) -> Result<(), axum::Error> {
    socket.send(Message::Text(text.into())).await
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use ha_core::{EventBus, StateMachine};

    use super::*;
    use crate::json::state_to_json;

    /// Les messages écrits directement donnent le même texte qu'avec l'ancien
    /// arbre `serde_json::Value`.
    #[test]
    fn direct_messages_write_the_same_text_as_the_value_tree() {
        let states = StateMachine::new(EventBus::new(16));
        let id: EntityId = "sensor.salon".parse().unwrap();
        let Value::Object(attributes) =
            json!({"unit_of_measurement": "°C", "friendly_name": "Salon \"sud\""})
        else {
            unreachable!()
        };
        let at = UNIX_EPOCH + Duration::from_micros(1_761_000_000_123_456);
        states
            .restore(id.clone(), "21.5", attributes, at, at)
            .unwrap();
        let current = states.get(&id).unwrap();

        let reply = to_text(
            7,
            &StatesReply {
                id: 7,
                result: vec![StateJson::new(&current)],
                success: true,
                kind: "result",
            },
        );
        let before = json!({"id": 7, "type": "result", "success": true, "result": [state_to_json(&current)]});
        assert_eq!(reply, before.to_string());

        for (old_state, new_state) in [
            (None, Some(Arc::clone(&current))),
            (Some(Arc::clone(&current)), None),
            (Some(Arc::clone(&current)), Some(Arc::clone(&current))),
        ] {
            let before = json!({
                "id": 3,
                "type": "event",
                "event": {
                    "event_type": "state_changed",
                    "data": {
                        "entity_id": id.as_str(),
                        "old_state": old_state.as_deref().map(state_to_json),
                        "new_state": new_state.as_deref().map(state_to_json),
                    },
                },
            });
            let event = Event::StateChanged {
                entity_id: id.clone(),
                old_state,
                new_state,
            };
            assert_eq!(event_message(3, &event), before.to_string());
        }
    }
}
