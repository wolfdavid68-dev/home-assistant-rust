//! Routes REST (`/api/...`).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ha_core::{Attributes, EntityId};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::ApiState;
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::json::{format_timestamp, state_to_json};

/// Nombre de lignes d'historique renvoyées par défaut.
const DEFAULT_HISTORY_LIMIT: u32 = 100;

/// Concept Rust : `#[serde(deny_unknown_fields)]` refuse un JSON qui contient
/// un champ inattendu. Une faute de frappe ou un champ malveillant donne une
/// erreur claire au lieu d'être ignoré en silence.
///
/// Pas de `#[derive(Debug)]` : le mot de passe ne doit jamais être affiché.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetStateRequest {
    state: String,
    #[serde(default)]
    attributes: Attributes,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryQuery {
    limit: Option<u32>,
}

/// Lit un corps JSON. On le fait nous-mêmes (au lieu de l'extracteur `Json`
/// d'axum) pour que toutes les erreurs aient le même format `{"error": …}`.
fn parse_json<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|error| ApiError::BadRequest(format!("JSON invalide : {error}")))
}

fn parse_entity_id(value: &str) -> Result<EntityId, ApiError> {
    value
        .parse()
        .map_err(|error: ha_core::CoreError| ApiError::BadRequest(error.to_string()))
}

/// `POST /api/auth/login` : `{"username", "password"}` → jeton d'accès.
pub(crate) async fn login(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let request: LoginRequest = parse_json(&body)?;
    // Le mot de passe sera effacé de la mémoire à la fin de la fonction.
    let password = Zeroizing::new(request.password);

    let issued = state
        .auth
        .login(&request.username, &password, client.ip())
        .await?;
    tracing::info!("connexion de {} depuis {}", request.username, client.ip());
    Ok(Json(json!({
        "access_token": issued.token.as_str(),
        "token_type": "Bearer",
        "expires_at": format_timestamp(issued.expires_at),
    })))
}

/// `POST /api/auth/logout` : révoque le jeton utilisé pour cette requête.
pub(crate) async fn logout(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<StatusCode, ApiError> {
    state.auth.revoke_token(&auth.token).await?;
    tracing::info!("déconnexion de {}", auth.user.username);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/` : vérifie que l'API répond et que le jeton est valide.
pub(crate) async fn root(auth: Authenticated) -> Json<Value> {
    Json(json!({
        "message": "API en marche",
        "version": ha_core::VERSION,
        "user": auth.user.username,
    }))
}

/// `GET /api/states` : tous les états.
pub(crate) async fn list_states(
    State(state): State<Arc<ApiState>>,
    _auth: Authenticated,
) -> Json<Value> {
    let states: Vec<Value> = state.core.states.all().iter().map(state_to_json).collect();
    Json(Value::Array(states))
}

/// `GET /api/states/{entity_id}`
pub(crate) async fn get_state(
    State(state): State<Arc<ApiState>>,
    _auth: Authenticated,
    Path(entity_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let current = state
        .core
        .states
        .get(&entity_id)
        .ok_or(ApiError::NotFound)?;
    Ok(Json(state_to_json(&current)))
}

/// `POST /api/states/{entity_id}` : `{"state": "on", "attributes": {…}}`.
/// Répond 201 si l'entité vient d'être créée, 200 sinon.
pub(crate) async fn set_state(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let request: SetStateRequest = parse_json(&body)?;

    let existed = state.core.states.get(&entity_id).is_some();
    state
        .core
        .states
        .set(entity_id.clone(), request.state, request.attributes)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    tracing::info!("{entity_id} modifié par {} via l'API", auth.user.username);

    let current = state
        .core
        .states
        .get(&entity_id)
        .ok_or(ApiError::NotFound)?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(state_to_json(&current))).into_response())
}

/// `DELETE /api/states/{entity_id}`
pub(crate) async fn delete_state(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    if !state.core.states.remove(&entity_id) {
        return Err(ApiError::NotFound);
    }
    tracing::info!("{entity_id} supprimé par {} via l'API", auth.user.username);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/history/{entity_id}?limit=100` : du plus récent au plus ancien.
pub(crate) async fn history(
    State(state): State<Arc<ApiState>>,
    _auth: Authenticated,
    Path(entity_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let limit = query.limit.unwrap_or(DEFAULT_HISTORY_LIMIT);
    let entries = state.store.history(&entity_id, limit).await?;

    let entries: Vec<Value> = entries
        .iter()
        .map(|entry| match &entry.state {
            Some(stored) => json!({
                "state": stored.state,
                "attributes": stored.attributes,
                "last_changed": format_timestamp(stored.last_changed),
                "last_updated": format_timestamp(stored.last_updated),
                "recorded_at": format_timestamp(entry.recorded_at),
            }),
            // Entité supprimée à ce moment-là.
            None => json!({
                "state": null,
                "recorded_at": format_timestamp(entry.recorded_at),
            }),
        })
        .collect();
    Ok(Json(
        json!({ "entity_id": entity_id.as_str(), "history": entries }),
    ))
}
