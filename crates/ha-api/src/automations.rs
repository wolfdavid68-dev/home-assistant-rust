//! Routes de l'éditeur d'automatisations (`/api/automations…`) et de la
//! confirmation d'identité (`/api/auth/confirm`).
//!
//! Lire et vérifier ne demande qu'une connexion. **Modifier** demande en plus
//! d'avoir retapé son mot de passe (et son code, si la double
//! authentification est activée) dans les 15 dernières minutes : un jeton
//! volé, ou un téléphone resté déverrouillé, ne suffit pas à installer une
//! automatisation qui ouvrirait les volets en pleine nuit.

use ha_auth::Permission;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use ha_automation::editor::{Entry, Source};
use ha_automation::{Automation, AutomationConfig, AutomationError, Editor, Trace, Traces};
use ha_core::{Attributes, EntityId};
use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::ApiState;
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::json::format_timestamp;
use crate::rest::{audit, parse_json};
use crate::roles;

/// Durée pendant laquelle une confirmation reste valable.
pub const CONFIRMATION_LIFETIME: Duration = Duration::from_mins(15);

/// Entité des notifications (voir `ha-notify`) : prévenue de chaque
/// modification, si elle existe.
const NOTIFY_ENTITY: &str = "notify.telephone";

/// L'éditeur, tel que l'API le reçoit.
#[derive(Debug)]
pub struct AutomationEditor {
    pub editor: Arc<Editor>,
    /// `false` : on peut consulter, mais pas modifier (`HA_AUTOMATION_EDITOR=off`).
    pub editable: bool,
    /// L'historique des exécutions (partagé avec le moteur).
    pub traces: Traces,
}

/// Pas de `#[derive(Debug)]` : le mot de passe ne doit jamais être affiché.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmRequest {
    password: String,
    #[serde(default)]
    code: Option<String>,
}

/// Une automatisation envoyée par le tableau de bord : soit en JSON (le
/// formulaire), soit en YAML (le mode texte).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AutomationRequest {
    #[serde(default)]
    config: Option<Value>,
    #[serde(default)]
    yaml: Option<String>,
}

impl AutomationRequest {
    fn into_config(self) -> Result<AutomationConfig, ApiError> {
        let config = match (self.config, self.yaml) {
            (Some(config), None) => AutomationConfig::from_json(config),
            (None, Some(yaml)) => AutomationConfig::from_yaml(&yaml),
            _ => {
                return Err(ApiError::BadRequest(
                    "il faut « config » (JSON) ou « yaml », pas les deux".into(),
                ));
            }
        };
        config.map_err(automation_error)
    }
}

fn automation_error(error: AutomationError) -> ApiError {
    match error {
        AutomationError::Storage(error) => ApiError::Internal(error.to_string()),
        AutomationError::Io(error) => ApiError::Internal(error.to_string()),
        other => ApiError::BadRequest(other.to_string()),
    }
}

fn editor(state: &ApiState) -> Result<&AutomationEditor, ApiError> {
    state.automations.as_ref().ok_or(ApiError::NotFound)
}

/// Temps de confirmation restant pour ce jeton.
pub(crate) fn confirmation_left(state: &ApiState, token: &str) -> Option<Duration> {
    let key = ha_auth::token::hash_token(token);
    let confirmations = state.confirmations.lock().ok()?;
    let until = confirmations.get(&key)?;
    until.checked_duration_since(Instant::now())
}

/// Refuse la requête si l'éditeur est en lecture seule, ou si l'identité n'a
/// pas été confirmée récemment.
fn require_confirmation(state: &ApiState, auth: &Authenticated) -> Result<Arc<Editor>, ApiError> {
    let automations = editor(state)?;
    if !automations.editable {
        return Err(ApiError::Forbidden(
            "l'éditeur est en lecture seule (HA_AUTOMATION_EDITOR=off)".into(),
        ));
    }
    if confirmation_left(state, &auth.token).is_none() {
        return Err(ApiError::ConfirmationRequired);
    }
    Ok(Arc::clone(&automations.editor))
}

/// Oublie la confirmation d'un jeton (à la déconnexion).
pub(crate) fn forget_confirmation(state: &ApiState, token: &str) {
    if let Ok(mut confirmations) = state.confirmations.lock() {
        confirmations.remove(&ha_auth::token::hash_token(token));
    }
}

/// `POST /api/auth/confirm` : `{"password", "code"}`. Valable 15 minutes,
/// pour ce jeton seulement.
pub(crate) async fn confirm(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let request: ConfirmRequest = parse_json(&body)?;
    let password = Zeroizing::new(request.password);
    let username = &auth.user.username;
    let result = state
        .auth
        .confirm(username, &password, request.code.as_deref(), client.ip())
        .await;
    if let Err(error) = result {
        if matches!(error, ha_auth::AuthError::InvalidCredentials) {
            audit(&state, username, "confirm_failed", None, None, client).await;
            // 403 et non 401 : le jeton est valide, c'est la confirmation qui
            // est refusée. Le tableau de bord ne doit pas croire la session
            // expirée.
            return Err(ApiError::Forbidden("mot de passe ou code incorrect".into()));
        }
        return Err(error.into());
    }
    let now = Instant::now();
    if let Ok(mut confirmations) = state.confirmations.lock() {
        // On en profite pour oublier les confirmations expirées.
        confirmations.retain(|_, until| *until > now);
        confirmations.insert(
            ha_auth::token::hash_token(&auth.token),
            now + CONFIRMATION_LIFETIME,
        );
    }
    audit(&state, username, "confirm", None, None, client).await;
    Ok(Json(
        json!({ "confirmed_for": CONFIRMATION_LIFETIME.as_secs() }),
    ))
}

/// Les entités citées qui n'existent pas (encore) : souvent une faute de frappe.
fn warnings(state: &ApiState, automation: &Automation) -> Vec<String> {
    automation
        .entity_ids()
        .into_iter()
        .filter(|entity_id| state.core.states.get(entity_id).is_none())
        .map(|entity_id| format!("{entity_id} n'existe pas (pour l'instant)"))
        .collect()
}

fn config_json(config: &AutomationConfig) -> Value {
    json!({
        "config": config.to_json(),
        "yaml": config.to_yaml(),
    })
}

fn entry_json(entry: &Entry) -> Value {
    let mut value = json!({
        "id": entry.id,
        "source": match entry.source {
            Source::File => "file",
            Source::Editor => "editor",
        },
        "alias": entry.config.as_ref().map_or(entry.id.as_str(), AutomationConfig::alias),
        "enabled": entry.config.as_ref().is_some_and(AutomationConfig::enabled),
        "config": entry.config.as_ref().map(AutomationConfig::to_json),
        "yaml": entry.config.as_ref().map(AutomationConfig::to_yaml),
        "error": entry.error,
        "updated_at": entry.updated_at.map(format_timestamp),
        "updated_by": entry.updated_by,
    });
    if entry.source == Source::File
        && let Some(object) = value.as_object_mut()
    {
        object.remove("updated_at");
        object.remove("updated_by");
    }
    value
}

/// `GET /api/automations` : toutes les automatisations (fichier et éditeur).
pub(crate) async fn list(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadAutomations)?;
    let automations = editor(&state)?;
    let entries = automations
        .editor
        .entries()
        .await
        .map_err(automation_error)?;
    let latest = automations.traces.latest();
    let list: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let mut value = entry_json(entry);
            if let Some(trace) = latest.get(&entry.id) {
                value["last_run"] = json!({
                    "started": trace.started.to_string(),
                    "outcome": trace.outcome.name(),
                });
            }
            value
        })
        .collect();
    Ok(Json(json!({
        "editable": automations.editable && auth.user.can(Permission::EditAutomations),
        "confirmed_for": confirmation_left(&state, &auth.token).map(|left| left.as_secs()),
        "automations": list,
    })))
}

/// `POST /api/automations/check` : vérifie sans enregistrer. Sert aussi à
/// passer du formulaire au YAML et inversement.
pub(crate) async fn check(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadAutomations)?;
    let editor = &editor(&state)?.editor;
    let config = parse_json::<AutomationRequest>(&body)?.into_config()?;
    let automation = editor.check(&config).map_err(automation_error)?;
    let mut response = config_json(&config);
    response["warnings"] = json!(warnings(&state, &automation));
    Ok(Json(response))
}

/// `PUT /api/automations/{id}` : crée ou modifie (confirmation requise).
pub(crate) async fn save(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    roles::require(&auth.user, Permission::EditAutomations)?;
    let editor = require_confirmation(&state, &auth)?;
    let config = parse_json::<AutomationRequest>(&body)?.into_config()?;
    if config.id() != id {
        return Err(ApiError::BadRequest(
            "l'identifiant ne correspond pas à l'adresse : pour renommer, crée une \
             nouvelle automatisation puis supprime l'ancienne"
                .into(),
        ));
    }
    let username = &auth.user.username;
    let saved = editor
        .save(&config, username)
        .await
        .map_err(automation_error)?;
    let (action, verb) = if saved.created {
        ("automation_create", "créée")
    } else {
        ("automation_update", "modifiée")
    };
    tracing::info!("automatisation « {id} » {verb} par {username}");
    audit(
        &state,
        username,
        action,
        Some(&id),
        Some(config.alias()),
        client,
    )
    .await;
    notify(
        &state,
        &format!("« {} » {verb} par {username}", config.alias()),
    );

    let mut response = config_json(&config);
    response["warnings"] = json!(warnings(&state, &saved.automation));
    let status = if saved.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(response)))
}

/// `DELETE /api/automations/{id}` (confirmation requise).
pub(crate) async fn delete(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    roles::require(&auth.user, Permission::EditAutomations)?;
    let editor = require_confirmation(&state, &auth)?;
    let username = &auth.user.username;
    if !editor
        .delete(&id, username)
        .await
        .map_err(automation_error)?
    {
        return Err(ApiError::NotFound);
    }
    tracing::info!("automatisation « {id} » supprimée par {username}");
    audit(
        &state,
        username,
        "automation_delete",
        Some(&id),
        None,
        client,
    )
    .await;
    notify(&state, &format!("« {id} » supprimée par {username}"));
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/automations/{id}/versions` : les versions précédentes.
pub(crate) async fn versions(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadAutomations)?;
    let editor = &editor(&state)?.editor;
    let versions: Vec<Value> = editor
        .versions(&id)
        .await
        .map_err(automation_error)?
        .iter()
        .map(|version| {
            json!({
                "version": version.version,
                "changed_at": format_timestamp(version.changed_at),
                "changed_by": version.changed_by,
                "deleted": version.config.is_none(),
                "config": version.config.as_ref().map(AutomationConfig::to_json),
                "yaml": version.config.as_ref().map(AutomationConfig::to_yaml),
            })
        })
        .collect();
    Ok(Json(json!({ "id": id, "versions": versions })))
}

fn trace_json(trace: &Trace) -> Value {
    json!({
        "run": trace.run,
        "alias": trace.alias,
        "started": trace.started.to_string(),
        "finished": trace.finished.map(|t| t.to_string()),
        "trigger": trace.trigger,
        "conditions": trace.conditions.iter().map(|check| json!({
            "description": check.description,
            "passed": check.passed,
        })).collect::<Vec<_>>(),
        "steps": trace.steps.iter().map(|step| json!({
            "path": step.path,
            "description": step.description,
            "at": step.at.to_string(),
            "status": step.status.name(),
            "detail": step.detail,
        })).collect::<Vec<_>>(),
        "truncated": trace.truncated,
        "outcome": trace.outcome.name(),
    })
}

/// `GET /api/automations/{id}/traces` : les dernières exécutions, la plus
/// récente en premier. Vide si l'automatisation ne s'est pas déclenchée
/// depuis le démarrage du serveur.
pub(crate) async fn traces(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadAutomations)?;
    let automations = editor(&state)?;
    let list: Vec<Value> = automations.traces.of(&id).iter().map(trace_json).collect();
    Ok(Json(json!({ "id": id, "traces": list })))
}

/// Prévient le téléphone d'une modification : si quelqu'un d'autre a pris la
/// main sur un compte, on le sait tout de suite.
fn notify(state: &ApiState, message: &str) {
    let Ok(entity_id) = NOTIFY_ENTITY.parse::<EntityId>() else {
        return;
    };
    if state.core.states.get(&entity_id).is_none() {
        return;
    }
    let mut data = Attributes::new();
    data.insert("title".into(), "Automatisation modifiée".into());
    data.insert("message".into(), message.into());
    if let Err(error) = state
        .core
        .call_service("notify", "send_message", entity_id, data)
    {
        tracing::warn!("notification impossible : {error}");
    }
}
