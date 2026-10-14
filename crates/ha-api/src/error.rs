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
    /// 404 : n'existe pas.
    NotFound,
    /// 429 : trop de tentatives.
    TooManyRequests { retry_after: Duration },
    /// 500 : erreur interne. Le détail va dans les logs, jamais dans la réponse.
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "authentification requise".into()),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "nom d'utilisateur ou mot de passe incorrect".into(),
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "introuvable".into()),
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
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<ha_storage::StorageError> for ApiError {
    fn from(error: ha_storage::StorageError) -> Self {
        Self::Internal(error.to_string())
    }
}
