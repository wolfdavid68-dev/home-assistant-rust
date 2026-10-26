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
//! | GET | `/api/automations/presets` | Les automatisations toutes prêtes (étape 85) et ce qui est coché |
//! | PUT | `/api/automations/presets/{name}` | En active une (ou la désactive) avec ses réglages |
//! | PUT | `/api/automations/{id}/layout` | Place des blocs dans le schéma |
//! | GET / PUT | `/api/home/drawing` | Dessin de la maison (style « plan ») : assemblage |
//! | GET / PUT / DELETE | `/api/home/images/{name}` | Ses images (fond, calques) |
//! | GET | `/api/push` | Notifications du navigateur : clé et appareils du compte |
//! | POST | `/api/push/subscriptions` | Abonne ce navigateur aux notifications |
//! | PATCH / DELETE | `/api/push/subscriptions/{id}` | Change ce qu'il reçoit / le retire |
//! | POST | `/api/push/test` | Envoie un essai à ses appareils |
//! | POST | `/api/conversation/process` | Assist : une phrase en français (« ferme le volet de la chambre »), `conversation_id` pour enchaîner (étape 96) |
//! | GET | `/api/conversation/voice` | L'écoute et la voix de la tablette sont-elles installées ? |
//! | POST | `/api/conversation/audio` | Une phrase dite à la tablette (WAV), écoutée puis comprise (étape 84) ; `?conversation_id=…` pour enchaîner |
//! | POST | `/api/conversation/speech` | La réponse lue par la voix de la maison (WAV) |
//! | POST | `/api/conversation/wake` | Un morceau de parole (WAV, 6 s au plus) : commence-t-il par le mot d'appel « Maison » ? (étape 97) |
//! | POST | `/api/mcp` | Un assistant IA branché par MCP (étape 55) |
//! | POST | `/api/push/action` | Un bouton touché sur une notification (sans connexion : le ticket en tient lieu) |
//! | GET | `/api/websocket` | API WebSocket (authentification dans le 1er message) |
//! | GET | `/` | Tableau de bord web (page publique ; les données demandent un jeton) |

mod assist;
mod auth;
mod automations;
mod confirm;
mod conversation;
mod drawing;
mod error;
mod json;
mod mcp;
mod presets;
mod push;
mod rest;
mod roles;
mod system;
mod tls;
mod voice;
mod wake;
mod web;
mod ws;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, StatusCode, header};
use axum::routing::{delete, get, post, put};
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
/// L'écoute d'une phrase : son tour, puis Whisper.
const VOICE_TIMEOUT: Duration = Duration::from_secs(90);

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
    /// Les boutons des notifications du navigateur (étape 51) : partagés
    /// avec le service qui les envoie.
    pub push_tickets: ha_push::Tickets,
    /// Les appels des assistants IA (MCP, étape 55), pour les limiter.
    mcp_limiter: mcp::Limiter,
    /// L'écoute des phrases dites à la tablette (étape 84), si installée.
    listening: OnceLock<ha_stt::Stt>,
    /// La voix qui lit les réponses à la tablette, si installée.
    speaking: OnceLock<ha_tts::Tts>,
    /// Deux réponses lues à la fois au plus.
    speech_turn: tokio::sync::Semaphore,
    /// Les conversations d'Assist en cours (étape 96), en mémoire.
    conversations: conversation::Conversations,
    /// Le mot d'appel « Maison » (étape 97) et la limite de ses morceaux.
    wake: wake::Wake,
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
            push_tickets: ha_push::Tickets::default(),
            mcp_limiter: mcp::Limiter::default(),
            listening: OnceLock::new(),
            speaking: OnceLock::new(),
            speech_turn: tokio::sync::Semaphore::new(2),
            conversations: conversation::Conversations::default(),
            wake: wake::Wake::default(),
            shutdown,
        })
    }

    /// Branche l'écoute et la voix de la tablette (étape 84). Sans elles,
    /// leurs routes répondent 503 et le bouton micro n'apparaît pas.
    pub fn set_voice(&self, listening: Option<ha_stt::Stt>, speaking: Option<ha_tts::Tts>) {
        if let Some(stt) = listening {
            let _ = self.listening.set(stt);
        }
        if let Some(tts) = speaking {
            let _ = self.speaking.set(tts);
        }
    }

    /// Règle le mot d'appel (étape 97, réglage `HA_MOT_APPEL`) : « Maison »
    /// sinon. À appeler au démarrage, avant de servir.
    pub fn set_wake_word(&self, word: &str) -> Result<(), String> {
        self.wake.set_word(word)
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
        .route("/api/auth/tablets", get(rest::list_tablets))
        .route("/api/auth/tablets/{id}", delete(rest::forget_tablet))
        .route("/api/auth/confirm", post(automations::confirm))
        .route("/api/automations", get(automations::list))
        .route("/api/automations/check", post(automations::check))
        .route("/api/automations/presets", get(presets::list))
        .route("/api/automations/presets/{name}", put(presets::save))
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
        .route("/api/camera_proxy/{entity_id}", get(rest::camera_image))
        .route("/api/media_cover/{key}", get(rest::media_cover))
        .route("/api/history/{entity_id}/series", get(rest::history_series))
        .route("/api/statistics/{entity_id}", get(rest::statistics))
        .route("/api/logbook", get(rest::logbook))
        .route("/api/energy", get(rest::energy))
        .route("/api/audit", get(rest::audit_log))
        .route("/api/areas", get(rest::areas))
        .route("/api/integrations", get(rest::integrations))
        .route("/api/approvals", get(rest::approvals))
        .route("/api/approvals/{id}", post(rest::answer_approval))
        .route("/api/system", get(system::system))
        .route("/api/push", get(push::overview))
        .route("/api/push/subscriptions", post(push::subscribe))
        .route(
            "/api/push/subscriptions/{id}",
            axum::routing::patch(push::update).delete(push::remove),
        )
        .route("/api/push/test", post(push::test))
        .route("/api/push/action", post(push::action))
        .route("/api/conversation/process", post(assist::process))
        .route("/api/conversation/voice", get(voice::overview))
        .route("/api/mcp", post(mcp::handle).get(mcp::no_stream))
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
        // Tableau de bord web (compressé, avec empreinte).
        .merge(web::router())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        // L'écoute et la voix prennent plus de temps (une autre phrase peut
        // passer avant) : leur propre délai, et une limite de taille pour le
        // son de 15 s.
        .route(
            "/api/conversation/audio",
            post(voice::listen)
                .layer::<_, std::convert::Infallible>(DefaultBodyLimit::max(ha_stt::MAX_WAV_BYTES))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    VOICE_TIMEOUT,
                )),
        )
        // Le mot d'appel (étape 97) : des morceaux de 6 s au plus.
        .route(
            "/api/conversation/wake",
            post(wake::wake)
                .layer::<_, std::convert::Infallible>(DefaultBodyLimit::max(wake::MAX_BYTES))
                .layer(TimeoutLayer::with_status_code(
                    StatusCode::REQUEST_TIMEOUT,
                    VOICE_TIMEOUT,
                )),
        )
        .route(
            "/api/conversation/speech",
            post(voice::speak).layer(TimeoutLayer::with_status_code(
                StatusCode::REQUEST_TIMEOUT,
                VOICE_TIMEOUT,
            )),
        )
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
        // La durée de chaque requête, pour la page « Système ».
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            system::measure,
        ))
        .with_state(state)
}

/// Sert l'API sur `listener` jusqu'à l'appel de [`ApiState::shutdown`].
pub use tls::{CA_PATH, Https, serve_https};

pub async fn serve(listener: TcpListener, state: Arc<ApiState>) -> std::io::Result<()> {
    let shutdown = state.shutdown_signal();
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
}
