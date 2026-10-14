//! Vérification du jeton d'accès sur chaque requête protégée.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::header;
use axum::http::request::Parts;
use ha_auth::User;
use zeroize::Zeroizing;

use crate::ApiState;
use crate::error::ApiError;

/// Preuve qu'une requête est authentifiée.
///
/// Concept Rust : c'est un « extracteur » axum. Il suffit qu'un gestionnaire
/// de route prenne un paramètre `Authenticated` pour que la route soit
/// protégée : axum appelle `from_request_parts` **avant** le gestionnaire, et
/// si le jeton est absent ou invalide, le gestionnaire n'est jamais exécuté.
/// Impossible d'oublier la vérification dans le corps de la fonction.
pub struct Authenticated {
    pub user: User,
    pub token: Zeroizing<String>,
}

impl FromRequestParts<Arc<ApiState>> for Authenticated {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        // En-tête attendu : `Authorization: Bearer hat_…`
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        let token = Zeroizing::new(token.trim().to_owned());

        let user = state
            .auth
            .authenticate(&token)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        Ok(Self { user, token })
    }
}
