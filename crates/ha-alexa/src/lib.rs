//! **Alexa en local**, comme `emulated_hue` dans Home Assistant.
//!
//! Le serveur se présente sur le réseau local comme un **pont Philips Hue**.
//! Un Echo sait piloter un pont Hue sans aucune « skill » ni compte : il le
//! découvre (SSDP, voir [`ssdp`]), puis lui envoie directement ses ordres en
//! HTTP (voir [`server`]). Aucun service cloud de notre côté, et aucun port
//! ouvert sur Internet. (La reconnaissance de la voix, elle, se fait toujours
//! chez Amazon : c'est le fonctionnement de tout Echo.)
//!
//! - [`config`] : le fichier `alexa.yaml`, la liste **explicite** des appareils ;
//! - [`hue`] : la traduction entre « lampe Hue » et nos appareils ;
//! - [`numbers`] : les numéros de lampes, stables, gardés dans la base.

pub mod config;
pub mod hue;
pub mod numbers;
pub mod server;
pub mod ssdp;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use ha_core::{Background, Core};
use ha_storage::Store;

pub use config::{Device, load, parse};
pub use server::{Allowed, Bridge, Identity, router};

/// Port HTTP par défaut. Les Echo récents ne parlent à un pont Hue que sur
/// le port 80.
pub const DEFAULT_PORT: u16 = 80;

#[derive(Debug, thiserror::Error)]
pub enum AlexaError {
    #[error("base de données : {0}")]
    Database(#[from] sqlx::Error),

    #[error("impossible d'écouter sur {address} : {source}")]
    Listen {
        address: SocketAddr,
        source: std::io::Error,
    },

    #[error("découverte (SSDP, port {port}) impossible : {source}")]
    Discovery { port: u16, source: std::io::Error },
}

/// Réglages.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Adresse de cette machine sur le réseau local (celle que l'Echo joindra).
    pub ip: Ipv4Addr,
    pub port: u16,
    /// Port de découverte (1900 ; autre chose seulement pour les tests).
    pub discovery_port: u16,
    pub allowed: Allowed,
    pub devices: Vec<Device>,
}

/// Le faux pont, en marche.
#[derive(Debug)]
pub struct Alexa {
    http: Background,
    discovery: Background,
    /// Numéro de lampe → appareil (pour les logs et les tests).
    pub lights: BTreeMap<u32, Device>,
    /// Adresse HTTP réellement utilisée.
    pub address: SocketAddr,
}

impl Alexa {
    /// Démarre le serveur HTTP et la découverte.
    pub async fn start(
        settings: Settings,
        store: Store,
        core: Arc<Core>,
    ) -> Result<Self, AlexaError> {
        let lights = numbers::assign(&store, &settings.devices).await?;
        let address = SocketAddr::new(IpAddr::V4(settings.ip), settings.port);
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|source| AlexaError::Listen { address, source })?;
        let address = listener
            .local_addr()
            .map_err(|source| AlexaError::Listen { address, source })?;
        let socket = ssdp::bind(settings.ip, settings.discovery_port).map_err(|source| {
            AlexaError::Discovery {
                port: settings.discovery_port,
                source,
            }
        })?;

        let identity = Identity::new(settings.ip, address.port());
        let allowed = Arc::new(settings.allowed.clone());
        let bridge = Arc::new(Bridge::new(
            core,
            store,
            lights.clone(),
            settings.allowed,
            identity.clone(),
        ));
        let app = router(bridge).into_make_service_with_connect_info::<SocketAddr>();
        let http = Background::spawn("Alexa (HTTP)", |token| async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(async move { token.cancelled().await })
                .await;
            if let Err(error) = served {
                tracing::error!("Alexa : le serveur s'est arrêté ({error})");
            }
        });
        let discovery = Background::spawn("Alexa (découverte)", |token| async move {
            tokio::select! {
                () = token.cancelled() => {}
                () = ssdp::serve(socket, identity, allowed) => {}
            }
        });
        tracing::info!(
            "Alexa : pont Hue sur http://{address}/, {} appareil(s) exposé(s)",
            lights.len()
        );
        Ok(Self {
            http,
            discovery,
            lights,
            address,
        })
    }

    pub async fn stop(self) {
        self.discovery.stop().await;
        self.http.stop().await;
    }
}
