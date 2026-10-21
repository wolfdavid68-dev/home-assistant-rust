//! Erreurs renvoyées par l'API, sous la forme `{"error": "message"}`.

use std::time::Duration;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    /// 400 : requête mal formée.
    BadRequest(String),
    /// 401 : pas de jeton, ou jeton invalide.
    Unauthorized,
    /// 401 aussi, mais pour un échec de connexion.
    InvalidCredentials,
    /// 403 : action interdite.
    Forbidden(String),
    /// 403 aussi : l'action demande de confirmer son identité (mot de passe).
    /// La réponse contient `"confirm": true`, pour que le tableau de bord
    /// affiche la demande de mot de passe.
    ConfirmationRequired,
    /// 404 : n'existe pas.
    NotFound,
    /// 429 : trop de tentatives.
    TooManyRequests { retry_after: Duration },
    /// 500 : erreur interne. Le détail va dans les logs, jamais dans la réponse.
    Internal(String),
    /// 502 : l'appareil (ou son pont) a refusé l'ordre.
    DeviceRefused(String),
    /// 503 : l'intégration de l'appareil est en panne ; l'ordre n'est pas envoyé.
    Unavailable(String),
    /// 504 : l'appareil n'a pas répondu à temps.
    DeviceTimeout(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "authentification requise".into()),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "nom d'utilisateur, mot de passe ou code incorrect".into(),
            ),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            Self::ConfirmationRequired => {
                return (
                    StatusCode::FORBIDDEN,
                    axum::Json(json!({
                        "error": "confirme ton identité avec ton mot de passe",
                        "confirm": true,
                    })),
                )
                    .into_response();
            }
            Self::NotFound => (StatusCode::NOT_FOUND, "introuvable".into()),
            Self::DeviceRefused(message) => (StatusCode::BAD_GATEWAY, message),
            Self::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
            Self::DeviceTimeout(message) => (StatusCode::GATEWAY_TIMEOUT, message),
            Self::TooManyRequests { retry_after } => {
                let mut response = (
                    StatusCode::TOO_MANY_REQUESTS,
                    axum::Json(json!({"error": "trop de tentatives, réessayer plus tard"})),
                )
                    .into_response();
                response.headers_mut().insert(
                    header::RETRY_AFTER,
                    HeaderValue::from(retry_after.as_secs()),
                );
                return response;
            }
            Self::Internal(detail) => {
                // Sécurité : un message d'erreur interne peut révéler des
                // chemins de fichiers, des requêtes SQL… On le garde pour nous.
                tracing::error!("erreur interne de l'API : {detail}");
                (StatusCode::INTERNAL_SERVER_ERROR, "erreur interne".into())
            }
        };
        let mut response = (status, axum::Json(json!({"error": message}))).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<ha_auth::AuthError> for ApiError {
    fn from(error: ha_auth::AuthError) -> Self {
        use ha_auth::AuthError;
        match error {
            AuthError::InvalidCredentials => Self::InvalidCredentials,
            AuthError::TooManyAttempts { retry_after } => Self::TooManyRequests { retry_after },
            error @ AuthError::InvalidWallLabel { .. } => Self::BadRequest(error.to_string()),
            error @ AuthError::AdminOnWall => Self::Forbidden(error.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<ha_storage::StorageError> for ApiError {
    fn from(error: ha_storage::StorageError) -> Self {
        Self::Internal(error.to_string())
    }
}
