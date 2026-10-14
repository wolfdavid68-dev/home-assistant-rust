//! La boucle réseau : connexion au broker, reconnexion automatique, et
//! exécution des [`Action`]s du pont.

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Core, Event};
use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Packet, QoS};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::MqttConfig;
use crate::bridge::{Action, Bridge};

/// Taille maximale d'un paquet reçu : au-delà, le broker est déconnecté.
const MAX_INCOMING_PACKET: usize = 256 * 1024;
const MAX_OUTGOING_PACKET: usize = 64 * 1024;
/// Délai maximal entre deux tentatives de reconnexion.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Poignée vers l'intégration MQTT qui tourne en arrière-plan.
#[derive(Debug)]
pub struct Mqtt {
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Mqtt {
    /// Démarre l'intégration. Elle se connecte, se reconnecte toute seule en
    /// cas de coupure, et tourne jusqu'à [`Mqtt::stop`].
    pub fn start(config: MqttConfig, core: Arc<Core>) -> Self {
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run(config, core, shutdown_rx));
        Self { shutdown, task }
    }

    /// Se déconnecte proprement du broker.
    pub async fn stop(self) {
        let _ = self.shutdown.send(());
        if let Err(error) = self.task.await {
            error!("l'intégration MQTT s'est arrêtée anormalement : {error}");
        }
    }
}

async fn run(config: MqttConfig, core: Arc<Core>, mut shutdown: oneshot::Receiver<()>) {
    let mut options = MqttOptions::new(&config.client_id, &config.host, config.port);
    options
        .set_keep_alive(Duration::from_secs(30))
        .set_max_packet_size(MAX_INCOMING_PACKET, MAX_OUTGOING_PACKET)
        .set_clean_session(true);
    if let Some(username) = &config.username {
        let password = config.password.as_deref().map_or("", String::as_str);
        options.set_credentials(username, password);
    }

    let (client, mut eventloop) = AsyncClient::new(options, 100);
    let mut bridge = Bridge::new(&config.discovery_prefix);
    let mut events = core.bus.subscribe();
    let mut backoff = Duration::from_secs(1);
    info!("MQTT : connexion à {}:{}", config.host, config.port);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,

            polled = eventloop.poll() => match polled {
                Ok(MqttEvent::Incoming(Packet::ConnAck(_))) => {
                    info!("MQTT : connecté à {}:{}", config.host, config.port);
                    backoff = Duration::from_secs(1);
                    // Session neuve à chaque connexion : on se réabonne à tout.
                    let topics = bridge
                        .discovery_filters()
                        .into_iter()
                        .chain(bridge.state_topics().cloned());
                    for topic in topics {
                        apply(&client, Action::Subscribe(topic));
                    }
                }
                Ok(MqttEvent::Incoming(Packet::Publish(message))) => {
                    for action in bridge.handle_message(&message.topic, &message.payload, &core.states) {
                        apply(&client, action);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    warn!("MQTT : connexion perdue ({error}), nouvel essai dans {} s", backoff.as_secs());
                    // On attend avant de réessayer, sans ignorer une demande d'arrêt.
                    tokio::select! {
                        () = tokio::time::sleep(backoff) => {}
                        _ = &mut shutdown => break,
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            },

            received = events.recv() => match received {
                Ok(event) => {
                    if let Event::CallService { domain, service, entity_id, data } = event.as_ref() {
                        for action in bridge.handle_service(domain, service, entity_id, data, &core.states) {
                            apply(&client, action);
                        }
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    warn!("MQTT : {missed} événements manqués");
                }
                Err(RecvError::Closed) => break,
            },
        }
    }

    let _ = client.try_disconnect();
    // Laisse le temps au message de déconnexion de partir.
    let _ = tokio::time::timeout(Duration::from_secs(1), eventloop.poll()).await;
    info!("MQTT : déconnecté");
}

/// Exécute une action sans bloquer : si la file d'envoi est pleine, l'action
/// est abandonnée avec un avertissement plutôt que de figer la boucle.
fn apply(client: &AsyncClient, action: Action) {
    let result = match &action {
        Action::Subscribe(topic) => client.try_subscribe(topic, QoS::AtMostOnce),
        Action::Unsubscribe(topic) => client.try_unsubscribe(topic),
        Action::Publish { topic, payload } => {
            client.try_publish(topic, QoS::AtLeastOnce, false, payload.as_bytes().to_vec())
        }
    };
    if let Err(error) = result {
        warn!("MQTT : action {action:?} impossible : {error}");
    }
}
