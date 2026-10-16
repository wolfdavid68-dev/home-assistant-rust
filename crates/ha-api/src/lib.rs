//! API HTTP (REST) et WebSocket, protégée par jeton d'accès.
//!
//! | Méthode | Chemin | Rôle |
//! |---------|--------|------|
//! | POST | `/api/auth/login` | Connexion : renvoie un jeton (seule route publique) |
//! | POST | `/api/auth/logout` | Révoque le jeton utilisé |
//! | POST | `/api/auth/confirm` | Confirme son identité avant une modification sensible |
//! | GET | `/api/` | Vérifie que l'API répond |
//! | GET | `/api/states` | Tous les états |
//! | GET / POST / DELETE | `/api/states/{entity_id}` | Lire / modifier / supprimer un état |
//! | GET | `/api/history/{entity_id}` | Historique d'une entité |
//! | GET | `/api/logbook` | Journal de bord (`?hours=24&entity_id=…`) |
//! | GET | `/api/statistics/{entity_id}` | Statistiques long terme (`?period=day&days=365`) |
//! | GET | `/api/energy` | Bilan énergétique : jours, mois, année, coûts (`?days=30`) |
//! | POST | `/api/services/{domain}/{service}` | Appel de service (`switch/turn_on`…) |
//! | GET | `/api/automations` | Automatisations (fichier et éditeur) |
//! | POST | `/api/automations/check` | Vérifie une automatisation sans l'enregistrer |
//! | PUT / DELETE | `/api/automations/{id}` | Crée, modifie, supprime (confirmation requise) |
//! | GET | `/api/automations/{id}/versions` | Versions précédentes |
//! | GET | `/api/automations/{id}/traces` | Dernières exécutions (historique) |
//! | PUT | `/api/automations/{id}/layout` | Place des blocs dans le schéma |
//! | GET / PUT | `/api/home/drawing` | Dessin de la maison (style « plan ») : assemblage |
//! | GET / PUT / DELETE | `/api/home/images/{name}` | Ses images (fond, calques) |
//! | GET | `/api/websocket` | API WebSocket (authentification dans le 1er message) |
//! | GET | `/` | Tableau de bord web (page publique ; les données demandent un jeton) |

mod auth;
mod automations;
mod drawing;
mod error;
mod json;
mod rest;
mod roles;
mod system;
mod web;
mod ws;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, StatusCode, header};
use axum::routing::{get, post, put};
use ha_auth::Auth;
use ha_core::CancellationToken;
use ha_core::Core;
use ha_storage::Store;
use tokio::net::TcpListener;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

pub use automations::{AutomationEditor, CONFIRMATION_LIFETIME};
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
    /// Les pièces de la maison (fichier `areas.yaml`).
    pub areas: Vec<ha_areas::Area>,
    /// L'éditeur d'automatisations (absent : les routes répondent 404).
    pub automations: Option<AutomationEditor>,
    /// Les compteurs du bilan énergétique (fichier `energy.yaml`).
    pub energy: ha_energy::EnergyConfig,
    /// Confirmations d'identité en cours : empreinte du jeton → fin de validité.
    confirmations: Mutex<HashMap<[u8; 32], Instant>>,
    /// Ce que le serveur consomme (page « Système »).
    metrics: system::Metrics,
    shutdown: CancellationToken,
}

impl ApiState {
    pub fn new(
        core: Arc<Core>,
        store: Store,
        auth: Arc<Auth>,
        areas: Vec<ha_areas::Area>,
        automations: Option<AutomationEditor>,
        energy: ha_energy::EnergyConfig,
    ) -> Arc<Self> {
        let shutdown = CancellationToken::new();
        Arc::new(Self {
            core,
            store,
            auth,
            areas,
            automations,
            energy,
            confirmations: Mutex::default(),
            metrics: system::Metrics::default(),
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
        .route("/api/auth/confirm", post(automations::confirm))
        .route("/api/automations", get(automations::list))
        .route("/api/automations/check", post(automations::check))
        .route(
            "/api/automations/{id}",
            put(automations::save).delete(automations::delete),
        )
        .route("/api/automations/{id}/versions", get(automations::versions))
        .route(
            "/api/automations/{id}/layout",
            put(automations::save_layout),
        )
        .route("/api/automations/{id}/traces", get(automations::traces))
        .route("/api/states", get(rest::list_states))
        .route(
            "/api/states/{entity_id}",
            get(rest::get_state)
                .post(rest::set_state)
                .delete(rest::delete_state),
        )
        .route("/api/history/{entity_id}", get(rest::history))
        .route("/api/history/{entity_id}/series", get(rest::history_series))
        .route("/api/statistics/{entity_id}", get(rest::statistics))
        .route("/api/logbook", get(rest::logbook))
        .route("/api/energy", get(rest::energy))
        .route("/api/audit", get(rest::audit_log))
        .route("/api/areas", get(rest::areas))
        .route("/api/integrations", get(rest::integrations))
        .route("/api/system", get(system::system))
        .route("/api/home/drawing", get(drawing::get).put(drawing::put))
        .route(
            "/api/home/images/{name}",
            get(drawing::get_image)
                .put(drawing::put_image)
                .delete(drawing::delete_image)
                // Une image dépasse les 64 Kio des autres requêtes. (La
                // limite la plus proche de la route l'emporte.)
                .layer(DefaultBodyLimit::max(drawing::MAX_IMAGE_SIZE)),
        )
        .route("/api/services/{domain}/{service}", post(rest::call_service))
        .route("/api/websocket", get(ws::handler))
        // Tableau de bord web.
        .route("/", get(web::index))
        .route("/app.js", get(web::app_js))
        .route("/canvas.js", get(web::canvas_js))
        .route("/house.js", get(web::house_js))
        .route("/style.css", get(web::style_css))
        .route("/icon.svg", get(web::icon_svg))
        .route("/house.svg", get(web::house_svg))
        .route("/fonts/geist.woff2", get(web::font_geist))
        .route("/fonts/geist-mono.woff2", get(web::font_geist_mono))
        .route("/fonts/bricolage-grotesque.woff2", get(web::font_bricolage))
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
        // La durée de chaque requête, pour la page « Système ».
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            system::measure,
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
