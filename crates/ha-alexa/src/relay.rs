//! Le **relais**, pour un serveur qui tourne dans Docker Desktop (Windows,
//! macOS).
//!
//! L'Echo cherche un pont par multicast (SSDP), qui ne traverse pas le réseau
//! virtuel de Docker Desktop. Il lui parle ensuite sur le port 80 de la
//! machine. Le relais tourne donc **sur la machine**, hors de Docker :
//!
//! - il répond à la découverte à la place du pont ;
//! - il reçoit l'Echo sur `ip:80` et recopie chaque connexion, telle quelle,
//!   vers le pont du conteneur (publié sur `127.0.0.1` seulement).
//!
//! Le relais ne comprend pas le langage Hue. Il filtre les adresses (seuls
//! les Echo autorisés passent, et lui voit leur vraie adresse, ce que le
//! conteneur ne peut pas), et limite le nombre et la durée des connexions.
//! Toutes les autres vérifications restent dans le pont.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use ha_core::Background;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::server::{Allowed, Identity};
use crate::{AlexaError, ssdp};

/// Connexions relayées en même temps, au plus. Un Echo en ouvre une ou deux.
pub const MAX_CONNECTIONS: usize = 16;
/// Durée maximale d'une connexion relayée.
pub const MAX_DURATION: Duration = Duration::from_secs(30);

/// Réglages du relais.
#[derive(Debug, Clone)]
pub struct RelaySettings {
    /// Adresse de cette machine sur le réseau local (celle que l'Echo joindra).
    pub ip: Ipv4Addr,
    /// Port où l'Echo se connecte (80).
    pub port: u16,
    /// Port de découverte (1900 ; autre chose seulement pour les tests).
    pub discovery_port: u16,
    /// Le pont, dans le conteneur (`127.0.0.1:8127`).
    pub target: SocketAddr,
    pub allowed: Allowed,
}

/// Le relais, en marche.
#[derive(Debug)]
pub struct Relay {
    forward: Background,
    discovery: Background,
    /// Adresse réellement utilisée côté Echo.
    pub address: SocketAddr,
}

impl Relay {
    /// Démarre la réception de l'Echo et la découverte.
    pub async fn start(settings: RelaySettings) -> Result<Self, AlexaError> {
        let address = SocketAddr::new(IpAddr::V4(settings.ip), settings.port);
        let listener = TcpListener::bind(address)
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

        // Même identité que le pont (même adresse, même port annoncés) : la
        // découverte et la fiche `description.xml` décrivent le même pont.
        let identity = Identity::new(settings.ip, address.port());
        let allowed = Arc::new(settings.allowed);
        let target = settings.target;
        let forward = Background::spawn("Alexa (relais)", {
            let allowed = Arc::clone(&allowed);
            |token| async move {
                tokio::select! {
                    () = token.cancelled() => {}
                    () = forward(listener, target, allowed) => {}
                }
            }
        });
        let discovery = Background::spawn("Alexa (découverte)", |token| async move {
            tokio::select! {
                () = token.cancelled() => {}
                () = ssdp::serve(socket, identity, allowed) => {}
            }
        });
        tracing::info!("Alexa : relais sur http://{address}/, vers le pont {target}");
        Ok(Self {
            forward,
            discovery,
            address,
        })
    }

    pub async fn stop(self) {
        self.discovery.stop().await;
        self.forward.stop().await;
    }
}

/// Accepte les connexions de l'Echo et les recopie vers le pont.
async fn forward(listener: TcpListener, target: SocketAddr, allowed: Arc<Allowed>) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (client, from) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::debug!("Alexa (relais) : connexion impossible ({error})");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let from = from.ip().to_canonical();
        if !allowed.permits(from) {
            tracing::warn!("Alexa (relais) : {from} refusée (pas dans HA_ALEXA_ALLOWED)");
            continue;
        }
        let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
            tracing::warn!("Alexa (relais) : trop de connexions, {from} refusée");
            continue;
        };
        tokio::spawn(async move {
            let _slot = slot;
            if tokio::time::timeout(MAX_DURATION, relay(client, from, target))
                .await
                .is_err()
            {
                tracing::debug!("Alexa (relais) : connexion de {from} trop longue, coupée");
            }
        });
    }
}

async fn relay(mut client: TcpStream, from: IpAddr, target: SocketAddr) {
    let mut bridge = match TcpStream::connect(target).await {
        Ok(bridge) => bridge,
        Err(error) => {
            tracing::warn!(
                "Alexa (relais) : le pont ne répond pas sur {target} ({error}). \
                 Le serveur tourne-t-il avec HA_ALEXA_LISTEN ?"
            );
            return;
        }
    };
    tracing::debug!("Alexa (relais) : {from} → {target}");
    if let Err(error) = tokio::io::copy_bidirectional(&mut client, &mut bridge).await {
        tracing::debug!("Alexa (relais) : connexion de {from} interrompue ({error})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn settings(target: SocketAddr, allowed: Allowed) -> RelaySettings {
        RelaySettings {
            ip: Ipv4Addr::LOCALHOST,
            port: 0,
            discovery_port: 0,
            target,
            allowed,
        }
    }

    /// Un faux pont qui répond « pont » à chaque connexion.
    async fn fake_bridge() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0u8; 64];
                let length = stream.read(&mut buffer).await.unwrap_or(0);
                let mut answer = b"pont:".to_vec();
                answer.extend_from_slice(&buffer[..length]);
                let _ = stream.write_all(&answer).await;
            }
        });
        address
    }

    async fn ask(relay: &Relay, message: &[u8]) -> Vec<u8> {
        let mut stream = TcpStream::connect(relay.address).await.unwrap();
        stream.write_all(message).await.unwrap();
        let mut answer = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut answer)).await;
        answer
    }

    #[tokio::test]
    async fn an_allowed_echo_reaches_the_bridge() {
        let bridge = fake_bridge().await;
        let relay = Relay::start(settings(bridge, Allowed::LocalNetwork))
            .await
            .unwrap();
        assert_eq!(ask(&relay, b"bonjour").await, b"pont:bonjour");
        relay.stop().await;
    }

    #[tokio::test]
    async fn other_addresses_are_cut_off() {
        let bridge = fake_bridge().await;
        let echo_only = Allowed::Only(vec!["192.168.1.40".parse().unwrap()]);
        let relay = Relay::start(settings(bridge, echo_only)).await.unwrap();
        // 127.0.0.1 n'est pas l'Echo : la connexion est fermée sans réponse.
        assert_eq!(ask(&relay, b"bonjour").await, b"");
        relay.stop().await;
    }

    #[tokio::test]
    async fn a_missing_bridge_closes_the_connection() {
        // Un port libéré aussitôt : personne n'y écoute.
        let nobody = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let relay = Relay::start(settings(nobody, Allowed::LocalNetwork))
            .await
            .unwrap();
        assert_eq!(ask(&relay, b"bonjour").await, b"");
        relay.stop().await;
    }
}
