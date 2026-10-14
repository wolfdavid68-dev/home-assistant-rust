//! La boucle principale : lecture des lampes, flux d'événements en direct,
//! envoi des commandes, et reconnexion automatique.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use ha_core::{Background, CancellationToken, Core, Event};
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, error, info, warn};

use crate::api::HueApi;
use crate::lights::Lights;
use crate::pairing::Pairing;
use crate::sse::SseParser;
use crate::tls;

/// Délai maximal entre deux tentatives de reconnexion.
const MAX_BACKOFF: Duration = Duration::from_mins(1);
/// Sans aucune donnée pendant ce temps, on considère le flux comme coupé
/// et on se reconnecte (ce qui relit aussi l'état de toutes les lampes).
const STREAM_IDLE: Duration = Duration::from_mins(10);

/// Poignée vers l'intégration Hue qui tourne en arrière-plan.
#[derive(Debug)]
pub struct Hue(Background);

impl Hue {
    pub fn start(pairing: Pairing, core: Arc<Core>) -> Self {
        Self(Background::spawn("l'intégration Hue", |token| {
            run(pairing, core, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(pairing: Pairing, core: Arc<Core>, token: CancellationToken) {
    let http = match tls::client(Some(pairing.fingerprint)) {
        Ok((http, _)) => http,
        Err(error) => {
            error!("Hue : impossible de préparer la connexion : {error}");
            return;
        }
    };
    let api = HueApi::new(http, &pairing.host, pairing.app_key);
    let mut lights = Lights::default();
    let mut events = core.bus.subscribe();
    let mut backoff = Duration::from_secs(1);
    info!(
        "Hue : connexion au pont {} ({})",
        pairing.bridge_id, pairing.host
    );

    // Concept Rust : `'connection:` est une étiquette de boucle. `break
    // 'connection` sort de cette boucle-là, même depuis une boucle imbriquée.
    'connection: loop {
        // 1. État complet de toutes les lampes, puis 2. flux des changements.
        let connected = async {
            lights.sync(api.lights().await?, &core.states);
            api.event_stream().await
        }
        .await;
        let response = match connected {
            Ok(response) => response,
            Err(error) => {
                warn!(
                    "Hue : pont injoignable ({error}), nouvel essai dans {} s",
                    backoff.as_secs()
                );
                tokio::select! {
                    () = tokio::time::sleep(backoff) => {}
                    () = token.cancelled() => break 'connection,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        info!("Hue : connecté, {} lampes", lights.len());
        backoff = Duration::from_secs(1);
        let mut stream = response.bytes_stream();
        let mut parser = SseParser::default();

        loop {
            tokio::select! {
                () = token.cancelled() => break 'connection,

                chunk = tokio::time::timeout(STREAM_IDLE, stream.next()) => {
                    let chunk = match chunk {
                        Ok(Some(Ok(chunk))) => chunk,
                        Ok(Some(Err(error))) => {
                            warn!("Hue : flux interrompu ({error})");
                            continue 'connection;
                        }
                        Ok(None) | Err(_) => {
                            debug!("Hue : flux terminé ou inactif, reconnexion");
                            continue 'connection;
                        }
                    };
                    let Ok(messages) = parser.push(&chunk) else {
                        warn!("Hue : flux illisible, reconnexion");
                        continue 'connection;
                    };
                    for message in messages {
                        if lights.apply_event(&message, &core.states) {
                            info!("Hue : lampes ajoutées ou supprimées, relecture");
                            continue 'connection;
                        }
                    }
                }

                received = events.recv() => match received {
                    Ok(event) => {
                        if let Event::CallService { domain, service, entity_id, data } = event.as_ref()
                            && let Some((id, body)) =
                                lights.command(domain, service, entity_id, data, &core.states)
                        {
                            // La commande part dans sa propre tâche : un pont lent
                            // ne bloque pas la réception des événements.
                            let api = api.clone();
                            let entity_id = entity_id.clone();
                            tokio::spawn(async move {
                                if let Err(error) = api.set_light(&id, &body).await {
                                    warn!("Hue : commande refusée pour {entity_id} : {error}");
                                }
                            });
                        }
                    }
                    Err(RecvError::Lagged(missed)) => warn!("Hue : {missed} événements manqués"),
                    Err(RecvError::Closed) => break 'connection,
                },
            }
        }
    }
    info!("Hue : arrêté");
}
