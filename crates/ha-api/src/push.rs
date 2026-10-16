//! Notifications sur le téléphone, par le navigateur (Web Push) : la clé du
//! serveur, et les appareils de chacun.
//!
//! | Méthode | Chemin | Rôle |
//! |---------|--------|------|
//! | GET | `/api/push` | Clé publique du serveur et appareils du compte |
//! | POST | `/api/push/subscriptions` | Abonne ce navigateur |
//! | PATCH / DELETE | `/api/push/subscriptions/{id}` | Change ce qu'il reçoit / le retire |
//! | POST | `/api/push/test` | Envoie un essai à ses propres appareils |
//!
//! Réservé aux comptes famille et administrateur : un invité ne reçoit pas
//! de notifications.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ha_auth::Permission;
use ha_core::{Attributes, EntityId, Origin};
use ha_push::{MAX_PER_USER, MAX_TOTAL, Subscription};
use ha_storage::NewPushSubscription;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ApiState;
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::json::format_timestamp;
use crate::rest::{audit, parse_json};
use crate::roles;

/// `GET /api/push`
pub(crate) async fn overview(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    let public_key = state.store.push_public_key().await?;
    let devices: Vec<Value> = state
        .store
        .push_subscriptions_of(auth.user.id)
        .await?
        .into_iter()
        .map(|device| {
            json!({
                "id": device.id,
                "name": device.name,
                // L'adresse est celle du navigateur de ce compte : la page
                // s'en sert pour reconnaître « cet appareil ».
                "endpoint": device.endpoint,
                "categories": device.categories,
                "created_at": format_timestamp(device.created_at),
                "last_success": device.last_success.map(format_timestamp),
                "failures": device.failures,
            })
        })
        .collect();
    Ok(Json(json!({
        "available": public_key.is_some(),
        "public_key": public_key,
        "devices": devices,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Keys {
    p256dh: String,
    auth: String,
}

/// Ce que donne `PushSubscription.toJSON()`, plus un nom et les catégories.
/// Pas de `Debug` : `auth` est un secret.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubscribeBody {
    endpoint: String,
    keys: Keys,
    #[serde(default, rename = "expirationTime")]
    _expiration_time: Option<Value>,
    #[serde(default)]
    name: String,
    #[serde(default = "all_categories")]
    categories: u8,
}

fn all_categories() -> u8 {
    ha_push::Category::ALL
}

/// `POST /api/push/subscriptions`
pub(crate) async fn subscribe(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Response, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    if state.store.push_public_key().await?.is_none() {
        return Err(ApiError::Unavailable(
            "les notifications du navigateur ne sont pas démarrées".into(),
        ));
    }
    let body: SubscribeBody = parse_json(&body)?;
    let subscription = Subscription::new(
        &body.endpoint,
        &body.keys.p256dh,
        &body.keys.auth,
        &body.name,
        body.categories,
    )
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    let mine = state
        .store
        .count_push_subscriptions(auth.user.id, &subscription.endpoint)
        .await?;
    if mine >= MAX_PER_USER {
        return Err(ApiError::Forbidden(format!(
            "{MAX_PER_USER} appareils au plus par compte : retires-en un d'abord"
        )));
    }
    if state.store.count_all_push_subscriptions().await? >= MAX_TOTAL {
        return Err(ApiError::Forbidden(format!(
            "{MAX_TOTAL} appareils au plus pour la maison"
        )));
    }
    let id = state
        .store
        .save_push_subscription(&NewPushSubscription {
            user_id: auth.user.id,
            endpoint: &subscription.endpoint,
            p256dh: &subscription.p256dh,
            auth: &subscription.auth,
            name: &subscription.name,
            categories: subscription.categories,
        })
        .await?;
    tracing::info!(
        "notifications du navigateur : « {} » ajouté par {}",
        subscription.name,
        auth.user.username
    );
    audit(
        &state,
        &auth.user.username,
        "push_subscribe",
        None,
        Some(&subscription.name),
        client,
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CategoriesBody {
    categories: u8,
}

/// `PATCH /api/push/subscriptions/{id}` avec `{"categories": 5}`
pub(crate) async fn update(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    Path(id): Path<i64>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    let CategoriesBody { categories } = parse_json(&body)?;
    ha_push::check_categories(categories)
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if !state
        .store
        .set_push_categories(auth.user.id, id, categories)
        .await?
    {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({ "id": id, "categories": categories })))
}

/// `DELETE /api/push/subscriptions/{id}`
pub(crate) async fn remove(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    if !state
        .store
        .delete_push_subscription(auth.user.id, id)
        .await?
    {
        return Err(ApiError::NotFound);
    }
    audit(
        &state,
        &auth.user.username,
        "push_unsubscribe",
        None,
        Some(&id.to_string()),
        client,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/push/test` : un essai, vers ses propres appareils seulement.
pub(crate) async fn test(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Response, ApiError> {
    roles::require(&auth.user, Permission::AllDevices)?;
    if state.store.push_public_key().await?.is_none() {
        return Err(ApiError::Unavailable(
            "les notifications du navigateur ne sont pas démarrées".into(),
        ));
    }
    let entity_id: EntityId = ha_push::ENTITY
        .parse()
        .map_err(|_| ApiError::Internal("entité notify.appli".into()))?;
    let mut data = Attributes::new();
    data.insert("title".into(), "Maison".into());
    data.insert(
        "message".into(),
        "Essai réussi : les notifications arrivent sur cet appareil.".into(),
    );
    data.insert("test".into(), true.into());
    state
        .core
        .call_service_from(
            Origin::User(auth.user.username.clone()),
            "notify",
            "send_message",
            entity_id,
            data,
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "result": "sent" }))).into_response())
}
