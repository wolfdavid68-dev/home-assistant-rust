//! Routes REST (`/api/...`).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ha_auth::Permission;
use ha_core::{Attributes, EntityId};
use ha_storage::AuditRecord;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::ApiState;
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::json::{StateJson, format_timestamp};
use crate::roles;

/// Nombre de lignes d'historique renvoyées par défaut.
const DEFAULT_HISTORY_LIMIT: u32 = 100;
/// Nombre de lignes du journal d'audit renvoyées par défaut.
const DEFAULT_AUDIT_LIMIT: u32 = 100;

/// Ajoute une ligne au journal d'audit.
///
/// Si l'écriture échoue (disque plein…), l'action n'est pas bloquée :
/// l'erreur est signalée dans les logs. Mieux vaut pouvoir encore fermer ses
/// volets qu'un journal parfait.
pub(crate) async fn audit(
    state: &ApiState,
    actor: &str,
    action: &str,
    target: Option<&str>,
    detail: Option<&str>,
    client: SocketAddr,
) {
    let source = client.ip().to_string();
    let record = AuditRecord {
        actor,
        action,
        target,
        detail,
        source: &source,
    };
    if let Err(error) = state.store.audit(record).await {
        tracing::error!("journal d'audit : écriture impossible ({error})");
    }
}

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
    /// Code de double authentification, si elle est activée pour ce compte.
    #[serde(default)]
    code: Option<String>,
    /// Le nom d'une tablette murale (étape 78) : la session reste alors
    /// ouverte tant que la tablette sert.
    #[serde(default)]
    wall_label: Option<String>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeriesQuery {
    hours: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogbookQuery {
    hours: Option<u32>,
    entity_id: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatisticsQuery {
    period: Option<String>,
    days: Option<u32>,
}

/// Période maximale des statistiques : 10 ans (31 jours par heure).
const MAX_STATISTICS_DAYS: u32 = 3660;

/// Lignes du journal de bord renvoyées par défaut.
const DEFAULT_LOGBOOK_LIMIT: u32 = 500;

/// Période par défaut d'un graphique, et période maximale (31 jours).
const DEFAULT_SERIES_HOURS: u32 = 24;
const MAX_SERIES_HOURS: u32 = 31 * 24;
/// Nombre de points d'une courbe : assez pour être fluide, peu pour être léger.
const SERIES_POINTS: u32 = 300;

/// Lit un corps JSON. On le fait nous-mêmes (au lieu de l'extracteur `Json`
/// d'axum) pour que toutes les erreurs aient le même format `{"error": …}`.
pub(crate) fn parse_json<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
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

    let code = request.code.as_deref();
    let result = match request.wall_label.as_deref() {
        Some(label) => {
            state
                .auth
                .login_wall(&request.username, &password, code, client.ip(), label)
                .await
        }
        None => {
            state
                .auth
                .login(&request.username, &password, code, client.ip())
                .await
        }
    };
    let issued = match result {
        Ok(issued) => issued,
        Err(error) => {
            // Les tentatives bloquées (trop d'échecs) ne sont pas journalisées :
            // elles ne coûtent rien à un attaquant, qui pourrait sinon
            // remplir le journal.
            if matches!(error, ha_auth::AuthError::InvalidCredentials) {
                audit(
                    &state,
                    &request.username,
                    "login_failed",
                    None,
                    None,
                    client,
                )
                .await;
            }
            return Err(error.into());
        }
    };
    let wall = request.wall_label.as_deref().map(str::trim);
    if let Some(label) = wall {
        audit(
            &state,
            &request.username,
            "login_wall",
            Some(label),
            None,
            client,
        )
        .await;
        tracing::info!(
            "tablette murale « {label} » connectée ({}) depuis {}",
            request.username,
            client.ip()
        );
    } else {
        audit(&state, &request.username, "login", None, None, client).await;
        tracing::info!("connexion de {} depuis {}", request.username, client.ip());
    }
    Ok(Json(json!({
        "access_token": issued.token.as_str(),
        "token_type": "Bearer",
        "expires_at": format_timestamp(issued.expires_at),
        "wall": wall.is_some(),
    })))
}

/// `GET /api/auth/tablets` : les tablettes murales connectées (étape 78),
/// pour un administrateur.
pub(crate) async fn list_tablets(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ManageTablets)?;
    let tablets: Vec<Value> = state
        .auth
        .wall_tablets()
        .await?
        .into_iter()
        .map(|tablet| {
            json!({
                "id": tablet.id,
                "label": tablet.label,
                "user": tablet.username,
                "created_at": format_timestamp(tablet.created_at),
                "last_used_at": format_timestamp(tablet.last_used_at),
            })
        })
        .collect();
    Ok(Json(Value::Array(tablets)))
}

/// `DELETE /api/auth/tablets/{id}` : déconnecte une tablette murale. Son
/// tableau de bord se ferme dans la minute (le WebSocket revérifie le jeton).
pub(crate) async fn forget_tablet(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    roles::require(&auth.user, Permission::ManageTablets)?;
    let label = state
        .auth
        .forget_wall_tablet(id)
        .await?
        .ok_or(ApiError::NotFound)?;
    audit(
        &state,
        &auth.user.username,
        "tablet_forget",
        Some(&label),
        None,
        client,
    )
    .await;
    tracing::info!(
        "tablette murale « {label} » déconnectée par {}",
        auth.user.username
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/auth/logout` : révoque le jeton utilisé pour cette requête.
pub(crate) async fn logout(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
) -> Result<StatusCode, ApiError> {
    state.auth.revoke_token(&auth.token).await?;
    crate::automations::forget_confirmation(&state, &auth.token);
    audit(&state, &auth.user.username, "logout", None, None, client).await;
    tracing::info!("déconnexion de {}", auth.user.username);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/` : vérifie que l'API répond et que le jeton est valide.
pub(crate) async fn root(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({
        "message": "API en marche",
        "version": ha_core::VERSION,
        "user": auth.user.username,
        "role": auth.user.role.as_str(),
        "role_label": auth.user.role.label(),
        "permissions": roles::permissions_json(&auth.user),
        // Cette session est-elle celle d'une tablette murale ? (son nom)
        "wall_label": state.auth.wall_label_of(&auth.token).await?,
    })))
}

/// `GET /api/states` : tous les états.
pub(crate) async fn list_states(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Response {
    let all = state.core.states.all();
    // Écrits directement depuis les états partagés, sans copie intermédiaire.
    let states: Vec<StateJson<'_>> = all
        .iter()
        .filter(|s| roles::can_see(&auth.user, s.entity_id(), Some(s)))
        .map(|s| StateJson::new(s))
        .collect();
    Json(states).into_response()
}

/// `GET /api/states/{entity_id}`
pub(crate) async fn get_state(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
) -> Result<Response, ApiError> {
    let entity_id = parse_entity_id(&entity_id)?;
    let current = state
        .core
        .states
        .get(&entity_id)
        // Caché : comme s'il n'existait pas (on ne révèle pas qu'il existe).
        .filter(|current| roles::can_see(&auth.user, &entity_id, Some(current)))
        .ok_or(ApiError::NotFound)?;
    Ok(Json(StateJson::new(&current)).into_response())
}

/// `GET /api/camera_proxy/{entity_id}` : la dernière image d'une caméra
/// (étape 82), comme dans Home Assistant. Une caméra cachée à ce compte (un
/// invité) est introuvable.
pub(crate) async fn camera_image(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
) -> Result<Response, ApiError> {
    use axum::http::{HeaderValue, header};
    let entity_id = parse_entity_id(&entity_id)?;
    if entity_id.domain() != "camera" {
        return Err(ApiError::NotFound);
    }
    let current = state.core.states.get(&entity_id);
    if !roles::can_see(&auth.user, &entity_id, current.as_deref()) || current.is_none() {
        return Err(ApiError::NotFound);
    }
    let image = state
        .core
        .images
        .get(&entity_id)
        .ok_or(ApiError::NotFound)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(image.content_type),
            ),
            // Une photo de la porte : jamais gardée par le navigateur.
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        Bytes::from_owner(image.bytes),
    )
        .into_response())
}

/// `GET /api/media_cover/{key}` : une pochette (étape 92), rangée par
/// l'intégration de musique. Elle ne change pas sous une même clé : le
/// navigateur peut la garder.
pub(crate) async fn media_cover(
    State(state): State<Arc<ApiState>>,
    _auth: Authenticated,
    Path(key): Path<String>,
) -> Result<Response, ApiError> {
    use axum::http::{HeaderValue, header};
    if !ha_core::covers::is_key(&key) {
        return Err(ApiError::NotFound);
    }
    let image = state.core.covers.get(&key).ok_or(ApiError::NotFound)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(image.content_type),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=86400, immutable"),
            ),
        ],
        Bytes::from_owner(image.bytes),
    )
        .into_response())
}

/// `POST /api/states/{entity_id}` : `{"state": "on", "attributes": {…}}`.
/// Répond 201 si l'entité vient d'être créée, 200 sinon.
pub(crate) async fn set_state(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    roles::require(&auth.user, Permission::WriteStates)?;
    let entity_id = parse_entity_id(&entity_id)?;
    let request: SetStateRequest = parse_json(&body)?;

    let existed = state.core.states.contains(&entity_id);
    state
        .core
        .states
        .set(entity_id.clone(), request.state, request.attributes)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    tracing::info!("{entity_id} modifié par {} via l'API", auth.user.username);
    let new_state = state.core.states.get(&entity_id);
    let detail = new_state.as_deref().map(ha_core::State::state);
    audit(
        &state,
        &auth.user.username,
        "state_set",
        Some(entity_id.as_str()),
        detail,
        client,
    )
    .await;

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
    Ok((status, Json(StateJson::new(&current))).into_response())
}

/// `DELETE /api/states/{entity_id}`
pub(crate) async fn delete_state(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    roles::require(&auth.user, Permission::WriteStates)?;
    let entity_id = parse_entity_id(&entity_id)?;
    if !state.core.states.remove(&entity_id) {
        return Err(ApiError::NotFound);
    }
    tracing::info!("{entity_id} supprimé par {} via l'API", auth.user.username);
    audit(
        &state,
        &auth.user.username,
        "state_delete",
        Some(entity_id.as_str()),
        None,
        client,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/history/{entity_id}?limit=100` : du plus récent au plus ancien.
pub(crate) async fn history(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadHistory)?;
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

/// `GET /api/logbook?hours=24&entity_id=light.salon` : le journal de bord,
/// du plus récent au plus ancien.
pub(crate) async fn logbook(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Query(query): Query<LogbookQuery>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadHistory)?;
    let hours = query.hours.unwrap_or(DEFAULT_SERIES_HOURS);
    if !(1..=MAX_SERIES_HOURS).contains(&hours) {
        return Err(ApiError::BadRequest(format!(
            "« hours » doit être entre 1 et {MAX_SERIES_HOURS}"
        )));
    }
    let entity_id = query
        .entity_id
        .as_deref()
        .map(parse_entity_id)
        .transpose()?;
    let until = SystemTime::now();
    let since = until
        .checked_sub(Duration::from_secs(u64::from(hours) * 3600))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let limit = query.limit.unwrap_or(DEFAULT_LOGBOOK_LIMIT);
    let entries: Vec<Value> = state
        .store
        .logbook(since, until, entity_id.as_ref(), limit)
        .await?
        .into_iter()
        .map(|entry| {
            json!({
                "at": format_timestamp(entry.at),
                "entity_id": entry.entity_id,
                "name": entry.name,
                "message": entry.message,
                "cause": entry.cause,
            })
        })
        .collect();
    Ok(Json(json!({ "entries": entries })))
}

/// `GET /api/statistics/{entity_id}?period=day&days=30` : les statistiques
/// long terme (moyenne, min, max, consommation), par heure, jour ou mois.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnergyQuery {
    #[serde(default)]
    days: Option<u32>,
}

/// `GET /api/energy?days=30` : le bilan énergétique (consommation et
/// production par jour et par mois, totaux, coûts).
pub(crate) async fn energy(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Query(query): Query<EnergyQuery>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadHistory)?;
    let days = query.days.unwrap_or(30);
    if !(1..=ha_energy::MAX_DAYS).contains(&days) {
        return Err(ApiError::BadRequest(format!(
            "« days » doit être entre 1 et {}",
            ha_energy::MAX_DAYS
        )));
    }
    let now = jiff::Zoned::now();
    let report = ha_energy::report(&state.store, &state.core.states, &state.energy, days, &now)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    serde_json::to_value(report)
        .map(Json)
        .map_err(|error| ApiError::Internal(error.to_string()))
}

pub(crate) async fn statistics(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
    Query(query): Query<StatisticsQuery>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadHistory)?;
    let entity_id = parse_entity_id(&entity_id)?;
    let (period, max_days) = match query.period.as_deref().unwrap_or("day") {
        "hour" => (ha_storage::Period::Hour, 31),
        "day" => (ha_storage::Period::Day, MAX_STATISTICS_DAYS),
        "month" => (ha_storage::Period::Month, MAX_STATISTICS_DAYS),
        _ => {
            return Err(ApiError::BadRequest(
                "« period » : hour, day ou month".into(),
            ));
        }
    };
    let days = query.days.unwrap_or(30);
    if !(1..=max_days).contains(&days) {
        return Err(ApiError::BadRequest(format!(
            "« days » doit être entre 1 et {max_days} pour cette période"
        )));
    }
    let until = SystemTime::now();
    let since = until
        .checked_sub(Duration::from_secs(u64::from(days) * 86_400))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let points: Vec<Value> = state
        .store
        .statistics(
            &entity_id,
            since,
            until,
            period,
            &jiff::tz::TimeZone::system(),
        )
        .await?
        .iter()
        .map(|p| {
            json!({
                "start": format_timestamp(p.start),
                "mean": p.mean,
                "min": p.min,
                "max": p.max,
                "change": p.change,
            })
        })
        .collect();
    Ok(Json(
        json!({ "entity_id": entity_id.as_str(), "points": points }),
    ))
}

/// `GET /api/history/{entity_id}/series?hours=24` : l'historique d'une
/// période, résumé pour un graphique (courbe et changements de valeur).
pub(crate) async fn history_series(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(entity_id): Path<String>,
    Query(query): Query<SeriesQuery>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadHistory)?;
    let entity_id = parse_entity_id(&entity_id)?;
    let hours = query.hours.unwrap_or(DEFAULT_SERIES_HOURS);
    if !(1..=MAX_SERIES_HOURS).contains(&hours) {
        return Err(ApiError::BadRequest(format!(
            "« hours » doit être entre 1 et {MAX_SERIES_HOURS}"
        )));
    }
    let until = SystemTime::now();
    let since = until
        .checked_sub(Duration::from_secs(u64::from(hours) * 3600))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let series = state
        .store
        .history_series(&entity_id, since, until, SERIES_POINTS)
        .await?;
    let numeric: Vec<Value> = series
        .numeric
        .iter()
        .map(|p| json!({"at": format_timestamp(p.at), "mean": p.mean, "min": p.min, "max": p.max}))
        .collect();
    let changes: Vec<Value> = series
        .changes
        .iter()
        .map(|c| json!({"at": format_timestamp(c.at), "state": c.state}))
        .collect();
    Ok(Json(json!({
        "entity_id": entity_id.as_str(),
        "since": format_timestamp(since),
        "until": format_timestamp(until),
        "numeric": numeric,
        "changes": changes,
    })))
}

/// `POST /api/services/{domain}/{service}` : `{"entity_id": "switch.prise", …paramètres}`.
///
/// Comme dans Home Assistant, la réponse arrive dès que l'ordre est transmis :
/// le nouvel état sera visible quand l'appareil l'aura confirmé.
pub(crate) async fn call_service(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path((domain, service)): Path<(String, String)>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let mut data: Attributes = parse_json(&body)?;
    let entity_id = match data.remove("entity_id") {
        Some(Value::String(entity_id)) => parse_entity_id(&entity_id)?,
        _ => return Err(ApiError::BadRequest("champ « entity_id » attendu".into())),
    };
    match execute(
        &state, &auth, client, &domain, &service, entity_id, data, false,
    )
    .await?
    {
        Outcome::Held(reason) => Ok((
            StatusCode::ACCEPTED,
            Json(json!({
                "result": "held",
                "message": format!("ordre en attente de validation : {reason}"),
            })),
        )
            .into_response()),
        Outcome::Sent => Ok(Json(json!({"result": "sent"})).into_response()),
        Outcome::Done => Ok(Json(json!({"result": "done"})).into_response()),
    }
}

/// Ce qu'est devenu un ordre accepté.
pub(crate) enum Outcome {
    /// Retenu par la garde : il attend qu'un membre de la famille le valide.
    Held(String),
    /// Envoyé ; l'appareil confirmera en changeant d'état.
    Sent,
    /// L'appareil a répondu qu'il l'a fait (Hue, Somfy).
    Done,
}

/// Donne un ordre au nom de `auth`, avec toutes les vérifications : droits
/// du compte, mot de passe redemandé pour ouvrir la maison, garde,
/// intégration en panne, réponse de l'appareil. Le chemin de
/// `POST /api/services`, d'Assist et des assistants IA (MCP).
///
/// `agent` : l'ordre vient d'un assistant IA (étape 55). Il ne peut jamais
/// ouvrir la maison, et la garde le retient quel que soit le rôle de son
/// compte, comme un ordre à la voix.
#[allow(
    clippy::too_many_arguments,
    reason = "le chemin commun de tous les ordres"
)]
pub(crate) async fn execute(
    state: &ApiState,
    auth: &Authenticated,
    client: SocketAddr,
    domain: &str,
    service: &str,
    entity_id: EntityId,
    data: Attributes,
    agent: bool,
) -> Result<Outcome, ApiError> {
    if !roles::can_control_with_members(&auth.user, &entity_id, &state.core.states) {
        tracing::warn!(
            "{domain}.{service} sur {entity_id} refusé à {} ({})",
            auth.user.username,
            auth.user.role.label()
        );
        return Err(ApiError::Forbidden(format!(
            "ton compte ({}) ne permet pas de commander cet appareil",
            auth.user.role.label()
        )));
    }
    // Un assistant IA ne touche jamais à ce qui ferme la maison : serrure,
    // alarme, sirène, porte de garage, portail, porte (même pour fermer).
    if agent && roles::guards_the_house(&entity_id, &state.core.states) {
        return Err(ApiError::Forbidden(
            "un assistant IA ne commande ni serrure, ni alarme, ni porte de garage, portail ou porte"
                .into(),
        ));
    }
    if roles::needs_confirmation(domain, service)
        && crate::automations::confirmation_left(state, &auth.token).is_none()
    {
        return Err(ApiError::ConfirmationRequired);
    }
    let origin = if agent {
        ha_core::Origin::Agent(auth.user.username.clone())
    } else {
        ha_core::Origin::User(auth.user.username.clone())
    };
    // Un invité ou un assistant IA, dans une pièce protégée (ou pendant les
    // heures calmes) : l'ordre attend qu'un membre de la famille le valide.
    if (agent || !auth.user.can(Permission::AllDevices))
        && let Some(reason) = state.core.guard_reason(&entity_id)
    {
        state
            .core
            .hold(
                origin,
                domain,
                service,
                entity_id.clone(),
                data,
                reason.clone(),
            )
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        tracing::info!(
            "{domain}.{service} sur {entity_id} retenu ({}) : {reason}",
            auth.user.username
        );
        audit(
            state,
            &auth.user.username,
            "held",
            Some(entity_id.as_str()),
            Some(&format!("{domain}.{service}")),
            client,
        )
        .await;
        return Ok(Outcome::Held(reason));
    }
    // L'intégration de l'appareil est en panne : l'ordre ne partirait nulle
    // part (et pourrait s'exécuter bien plus tard). On le dit tout de suite.
    if let Some(down) = state.core.owner_down(&entity_id) {
        return Err(ApiError::Unavailable(format!(
            "{} ne répond plus depuis {} : l'ordre n'a pas été envoyé",
            down.name,
            clock(down.since)
        )));
    }
    // Hue, Somfy : on attend de savoir si l'appareil a accepté.
    let answering = state.core.answering_owner(&entity_id);
    let reply = if answering.is_some() {
        Some(
            state
                .core
                .call_service_tracked(origin, domain, service, entity_id.clone(), data)
                .map_err(|error| ApiError::BadRequest(error.to_string()))?,
        )
    } else {
        state
            .core
            .call_service_from(origin, domain, service, entity_id.clone(), data)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        None
    };
    tracing::info!(
        "{domain}.{service} sur {entity_id} demandé par {}",
        auth.user.username
    );
    audit(
        state,
        &auth.user.username,
        "service",
        Some(entity_id.as_str()),
        Some(&format!("{domain}.{service}")),
        client,
    )
    .await;
    let (Some((id, reply)), Some(owner)) = (reply, answering) else {
        return Ok(Outcome::Sent);
    };
    // Un peu plus que la durée de vie de l'ordre : reçu à temps, il a encore
    // quelques secondes pour arriver jusqu'à l'appareil.
    match tokio::time::timeout(ha_core::COMMAND_TTL + ANSWER_MARGIN, reply).await {
        Ok(Ok(Ok(()))) => Ok(Outcome::Done),
        Ok(Ok(Err(refused))) => Err(ApiError::DeviceRefused(refused)),
        Ok(Err(_)) => Err(ApiError::DeviceRefused(format!(
            "{} n'a pas répondu",
            owner.name
        ))),
        Err(_) => {
            state.core.forget_command(id);
            Err(ApiError::DeviceTimeout(format!(
                "{} n'a pas répondu à temps",
                owner.name
            )))
        }
    }
}

/// `GET /api/approvals` : les ordres retenus par la garde, qui attendent
/// d'être validés.
pub(crate) async fn approvals(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    let list: Vec<Value> = state
        .core
        .held_orders()
        .into_iter()
        .map(|order| {
            json!({
                "id": order.id,
                "domain": order.domain,
                "service": order.service,
                "entity_id": order.entity_id.as_str(),
                "by": order.origin.describe(),
                "reason": order.reason,
                "held_since": format_timestamp(order.held_since),
            })
        })
        .collect();
    Ok(Json(json!({ "approvals": list })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApprovalBody {
    approve: bool,
}

/// `POST /api/approvals/{id}` avec `{"approve": true}` : valide un ordre
/// retenu (il part, comme s'il venait d'être donné) ou le refuse.
pub(crate) async fn answer_approval(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(id): Path<u64>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    let ApprovalBody { approve } = parse_json(&body)?;
    let order = state.core.release(id).ok_or(ApiError::NotFound)?;
    decide(&state, order, approve, &auth.user.username, client).await?;
    Ok(Json(
        json!({ "result": if approve { "sent" } else { "refused" } }),
    ))
}

/// Valide (l'ordre part) ou refuse un ordre retenu, déjà retiré de
/// l'attente : depuis la cloche, ou depuis un bouton de notification.
pub(crate) async fn decide(
    state: &ApiState,
    order: ha_core::HeldOrder,
    approve: bool,
    username: &str,
    client: SocketAddr,
) -> Result<(), ApiError> {
    // Les boutons envoyés aux autres appareils ne servent plus.
    state.push_tickets.forget_approval(order.id);
    let call = format!("{}.{}", order.domain, order.service);
    let by = order.origin.describe().unwrap_or_default();
    let answered = order.clone();
    if approve {
        // L'ordre garde son origine (« à la voix ») : le journal de bord dira
        // qui l'a donné ; le journal d'audit, qui l'a validé.
        state
            .core
            .call_service_from(
                order.origin,
                &order.domain,
                &order.service,
                order.entity_id.clone(),
                order.data,
            )
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    }
    // Les téléphones remplacent leur « Ordre à valider » par la réponse.
    state.core.approval_answered(&answered, approve, username);
    let verdict = if approve { "validé" } else { "refusé" };
    tracing::info!("{call} sur {} {verdict} par {username}", order.entity_id);
    audit(
        state,
        username,
        if approve { "approve" } else { "deny" },
        Some(order.entity_id.as_str()),
        Some(format!("{call} {by}").trim_end()),
        client,
    )
    .await;
    Ok(())
}

/// Le temps laissé à l'appareil, au-delà de la durée de vie de l'ordre.
const ANSWER_MARGIN: Duration = Duration::from_secs(3);

/// « 10:32 » (heure locale), pour un message.
fn clock(time: SystemTime) -> String {
    jiff::Timestamp::try_from(time).map_or_else(
        |_| "un moment".to_owned(),
        |t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%H:%M")
                .to_string()
        },
    )
}

/// `GET /api/audit?limit=100` : le journal d'audit, du plus récent au plus ancien.
pub(crate) async fn audit_log(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadAudit)?;
    let limit = query.limit.unwrap_or(DEFAULT_AUDIT_LIMIT);
    let entries: Vec<Value> = state
        .store
        .audit_log(limit)
        .await?
        .into_iter()
        .map(|entry| {
            json!({
                "id": entry.id,
                "at": format_timestamp(entry.at),
                "actor": entry.actor,
                "action": entry.action,
                "target": entry.target,
                "detail": entry.detail,
                "source": entry.source,
            })
        })
        .collect();
    Ok(Json(json!({ "entries": entries })))
}

/// `GET /api/integrations` : la santé de chaque intégration (Hue, Somfy,
/// MQTT…). Réservé à ceux qui voient tous les appareils : les erreurs peuvent
/// citer une adresse du réseau.
pub(crate) async fn integrations(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    let list: Vec<Value> = state
        .core
        .integrations()
        .into_iter()
        .map(|status| {
            let (error, retry_at) = match &status.health {
                ha_core::Health::Retrying { error, retry_at }
                | ha_core::Health::Crashed { error, retry_at } => {
                    (Some(error.clone()), Some(format_timestamp(*retry_at)))
                }
                _ => (None, None),
            };
            json!({
                "id": status.id,
                "name": status.name,
                "status": status.health.name(),
                "since": format_timestamp(status.since),
                "error": error,
                "retry_at": retry_at,
                "restarts": status.restarts,
                "unavailable": status.unavailable,
                "entities": status.entities,
            })
        })
        .collect();
    Ok(Json(json!({ "integrations": list })))
}

/// `GET /api/areas` : les pièces de la maison et leurs appareils.
pub(crate) async fn areas(State(state): State<Arc<ApiState>>, auth: Authenticated) -> Json<Value> {
    let mut areas: Vec<(String, Vec<Value>)> = state
        .areas
        .iter()
        .map(|area| {
            let entities: Vec<Value> = area
                .entities
                .iter()
                .filter(|entity| {
                    let current = state.core.states.get(&entity.entity_id);
                    roles::can_see(&auth.user, &entity.entity_id, current.as_deref())
                })
                .map(|entity| json!({"entity_id": entity.entity_id.as_str(), "name": entity.name}))
                .collect();
            (area.name.clone(), entities)
        })
        .collect();
    // Les appareils dont l'intégration donne la pièce (SmartThings, étape 86)
    // et qui ne sont pas dans areas.yaml : dans la pièce du même nom, ou
    // dans une nouvelle pièce, à la fin.
    let placed: std::collections::HashSet<&EntityId> = state
        .areas
        .iter()
        .flat_map(|area| area.entities.iter().map(|e| &e.entity_id))
        .collect();
    // `all()` les donne déjà triés par identifiant.
    for entity in state.core.states.all() {
        let Some(room) = entity
            .attributes()
            .get("piece")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|room| !room.is_empty())
        else {
            continue;
        };
        if placed.contains(entity.entity_id())
            || !roles::can_see(&auth.user, entity.entity_id(), Some(&entity))
        {
            continue;
        }
        let key = ha_assist::normalize(room);
        let item = json!({"entity_id": entity.entity_id().as_str(), "name": Value::Null});
        if let Some(index) = areas
            .iter()
            .position(|(name, _)| ha_assist::normalize(name) == key)
        {
            areas[index].1.push(item);
        } else if areas.len() < 100 {
            areas.push((room.chars().take(64).collect(), vec![item]));
        }
    }
    let areas: Vec<Value> = areas
        .into_iter()
        .map(|(name, entities)| json!({"name": name, "entities": entities}))
        .collect();
    Json(json!({ "areas": areas }))
}
