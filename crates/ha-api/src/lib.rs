//! API HTTP (REST) et WebSocket, protégée par jeton d'accès.
//!
//! | Méthode | Chemin | Rôle |
//! |---------|--------|------|
//! | POST | `/api/auth/login` | Connexion : renvoie un jeton (seule route publique) |
//! | POST | `/api/auth/logout` | Révoque le jeton utilisé |
//! | GET | `/api/` | Vérifie que l'API répond |
//! | GET | `/api/states` | Tous les états |
//! | GET / POST / DELETE | `/api/states/{entity_id}` | Lire / modifier / supprimer un état |
//! | GET | `/api/history/{entity_id}` | Historique d'une entité |
//! | POST | `/api/services/{domain}/{service}` | Appel de service (`switch/turn_on`…) |
//! | GET | `/api/websocket` | API WebSocket (authentification dans le 1er message) |
//! | GET | `/` | Tableau de bord web (page publique ; les données demandent un jeton) |

mod auth;
mod error;
mod json;
mod rest;
mod web;
mod ws;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use ha_auth::Auth;
use ha_core::CancellationToken;
use ha_core::Core;
use ha_storage::Store;
use tokio::net::TcpListener;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

pub use json::{format_timestamp, state_to_json};

/// Taille maximale du corps d'une requête : 64 Kio suffisent largement.
const MAX_BODY_SIZE: usize = 64 * 1024;
/// Une requête HTTP qui dure plus longtemps est interrompue.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Ce que les routes de l'API partagent.
#[derive(Debug)]
pub struct ApiState {
    pub core: Arc<Core>,
    pub store: Store,
    pub auth: Arc<Auth>,
    shutdown: CancellationToken,
}

impl ApiState {
    pub fn new(core: Arc<Core>, store: Store, auth: Arc<Auth>) -> Arc<Self> {
        let shutdown = CancellationToken::new();
        Arc::new(Self {
            core,
            store,
            auth,
            shutdown,
        })
    }

    /// Demande l'arrêt du serveur : plus de nouvelles connexions, et les
    /// WebSockets ouverts sont fermés.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    fn shutdown_signal(&self) -> CancellationToken {
        self.shutdown.clone()
    }
}

/// Toutes les routes de l'API.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/api/", get(rest::root))
        .route("/api/auth/login", post(rest::login))
        .route("/api/auth/logout", post(rest::logout))
        .route("/api/states", get(rest::list_states))
        .route(
            "/api/states/{entity_id}",
            get(rest::get_state)
                .post(rest::set_state)
                .delete(rest::delete_state),
        )
        .route("/api/history/{entity_id}", get(rest::history))
        .route("/api/audit", get(rest::audit_log))
        .route("/api/services/{domain}/{service}", post(rest::call_service))
        .route("/api/websocket", get(ws::handler))
        // Tableau de bord web.
        .route("/", get(web::index))
        .route("/app.js", get(web::app_js))
        .route("/style.css", get(web::style_css))
        .route("/icon.svg", get(web::icon_svg))
        .route("/manifest.webmanifest", get(web::manifest))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        // En-têtes de sécurité sur toutes les réponses.
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(web::CONTENT_SECURITY_POLICY),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .with_state(state)
}

/// Sert l'API sur `listener` jusqu'à l'appel de [`ApiState::shutdown`].
pub async fn serve(listener: TcpListener, state: Arc<ApiState>) -> std::io::Result<()> {
    let shutdown = state.shutdown_signal();
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
}
