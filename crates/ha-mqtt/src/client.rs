//! La boucle réseau : connexion au broker, reconnexion automatique, et
//! exécution des [`Action`]s du pont.

use ha_core::Health;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use ha_core::{Background, Backoff, CancellationToken, Core, Event};
use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Packet, QoS};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

use crate::MqttConfig;
use crate::bridge::{Action, Bridge};

/// Taille maximale d'un paquet reçu : au-delà, le broker est déconnecté.
/// Assez pour la photo d'une sonnette (2 Mio, 2,7 en base64), étape 82.
const MAX_INCOMING_PACKET: usize = 3 * 1024 * 1024;
const MAX_OUTGOING_PACKET: usize = 64 * 1024;
/// Délai maximal entre deux tentatives de reconnexion.
const MAX_BACKOFF: Duration = Duration::from_mins(1);

/// Poignée vers l'intégration MQTT qui tourne en arrière-plan.
#[derive(Debug)]
pub struct Mqtt(Background);

impl Mqtt {
    /// Démarre l'intégration. Elle se connecte, se reconnecte toute seule en
    /// cas de coupure, et tourne jusqu'à [`Mqtt::stop`].
    pub fn start(config: MqttConfig, core: Arc<Core>) -> Self {
        // Sous surveillance : si elle plante, elle est relancée.
        let supervisor = Arc::clone(&core);
        Self(ha_core::supervise(
            supervisor,
            crate::ID,
            crate::NAME,
            move |token| run(config.clone(), Arc::clone(&core), token),
        ))
    }

    /// Se déconnecte proprement du broker.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(config: MqttConfig, core: Arc<Core>, token: CancellationToken) {
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
    let mut bridge = Bridge::new(&config.discovery_prefix).with_images(Arc::clone(&core.images));
    // Les Shelly récents, sans découverte (étape 66).
    let mut shellies = crate::shelly::Shellies::default();
    let mut events = core.bus.subscribe();
    let mut backoff = Backoff::new(Duration::from_secs(1), MAX_BACKOFF);
    info!("MQTT : connexion à {}:{}", config.host, config.port);

    loop {
        tokio::select! {
            () = token.cancelled() => break,

            polled = eventloop.poll() => match polled {
                Ok(MqttEvent::Incoming(Packet::ConnAck(_))) => {
                    info!("MQTT : connecté à {}:{}", config.host, config.port);
                    core.report(crate::ID, crate::NAME, Health::Connected);
                    backoff.reset();
                    // Session neuve à chaque connexion : on se réabonne à tout.
                    let topics = bridge
                        .discovery_filters()
                        .into_iter()
                        .chain(crate::shelly::FILTERS.iter().map(|f| (*f).to_owned()))
                        .chain(bridge.state_topics().cloned());
                    for topic in topics {
                        apply(&client, &Action::Subscribe(topic));
                    }
                    // Après les abonnements, pour recevoir les descriptions
                    // que les ponts renvoient en réponse.
                    apply(&client, &bridge.birth());
                }
                Ok(MqttEvent::Incoming(Packet::Publish(message))) => {
                    // Un Shelly d'abord ; le pont de découverte voit quand
                    // même le message (il ignore ce qu'il ne suit pas).
                    let shelly = shellies
                        .handle_message(&message.topic, &message.payload, &core.states)
                        .unwrap_or_default();
                    for action in &shelly {
                        apply(&client, action);
                    }
                    for action in bridge.handle_message(&message.topic, &message.payload, &core.states) {
                        if let Action::Fire { event_type, data } = action {
                            if let Err(error) = core.fire(&event_type, data) {
                                warn!("MQTT : événement refusé ({error})");
                            }
                        } else {
                            apply(&client, &action);
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    let wait = backoff.next();
                    warn!("MQTT : connexion perdue ({error}), nouvel essai dans {} s", wait.as_secs());
                    core.report(crate::ID, crate::NAME, Health::Retrying {
                        error: format!("broker injoignable ({error})"),
                        retry_at: SystemTime::now() + wait,
                    });
                    // On attend avant de réessayer, sans ignorer une demande d'arrêt.
                    tokio::select! {
                        () = tokio::time::sleep(wait) => {}
                        () = token.cancelled() => break,
                    }
                }
            },

            received = events.recv() => match received {
                Ok(event) => {
                    if let Event::CallService { domain, service, entity_id, data, issued_at, .. } = event.as_ref() {
                        // Un ordre resté en attente pendant une coupure n'est
                        // plus voulu : on ne l'envoie pas.
                        if ha_core::commands::is_stale(*issued_at) {
                            warn!("MQTT : ordre périmé ignoré pour {entity_id}");
                            continue;
                        }
                        for action in bridge.handle_service(domain, service, entity_id, data, &core.states) {
                            apply(&client, &action);
                        }
                        for action in shellies.handle_service(domain, service, entity_id) {
                            apply(&client, &action);
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
fn apply(client: &AsyncClient, action: &Action) {
    let result = match action {
        Action::Subscribe(topic) => client.try_subscribe(topic, QoS::AtMostOnce),
        Action::Unsubscribe(topic) => client.try_unsubscribe(topic),
        Action::Publish { topic, payload } => {
            client.try_publish(topic, QoS::AtLeastOnce, false, payload.as_bytes().to_vec())
        }
        // Traité par la boucle (il faut le noyau).
        Action::Fire { .. } => return,
    };
    if let Err(error) = result {
        warn!("MQTT : action {action:?} impossible : {error}");
    }
}
