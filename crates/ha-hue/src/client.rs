//! La boucle principale : lecture des lampes, flux d'événements en direct,
//! envoi des commandes, et reconnexion automatique.

use ha_core::Health;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use futures_util::StreamExt;
use ha_core::{Background, Backoff, CancellationToken, Core, Event};
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
        // Sous surveillance : si elle plante, elle est relancée.
        let supervisor = Arc::clone(&core);
        // Hue dit si le pont a accepté chaque ordre.
        core.answers_commands(crate::ID);
        Self(ha_core::supervise(
            supervisor,
            crate::ID,
            crate::NAME,
            move |token| run(pairing.clone(), Arc::clone(&core), token),
        ))
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
    let mut backoff = Backoff::new(Duration::from_secs(1), MAX_BACKOFF);
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
                let wait = backoff.next();
                warn!(
                    "Hue : pont injoignable ({error}), nouvel essai dans {} s",
                    wait.as_secs()
                );
                core.report(
                    crate::ID,
                    crate::NAME,
                    Health::Retrying {
                        error: format!("pont injoignable ({error})"),
                        retry_at: SystemTime::now() + wait,
                    },
                );
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = token.cancelled() => break 'connection,
                }
                continue;
            }
        };
        info!("Hue : connecté, {} lampes", lights.len());
        core.report(crate::ID, crate::NAME, Health::Connected);
        backoff.reset();
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
                        if let Event::CallService { domain, service, entity_id, data, command, issued_at, .. } = event.as_ref()
                            && let Some((id, body)) =
                                lights.command(domain, service, entity_id, data, &core.states)
                        {
                            // Un ordre resté en attente pendant une coupure n'est
                            // plus voulu : on ne l'exécute pas.
                            if ha_core::commands::is_stale(*issued_at) {
                                warn!("Hue : ordre périmé ignoré pour {entity_id}");
                                core.command_done(*command, Err("ordre périmé : le pont a mis trop longtemps".into()));
                                continue;
                            }
                            // La commande part dans sa propre tâche : un pont lent
                            // ne bloque pas la réception des événements.
                            let api = api.clone();
                            let entity_id = entity_id.clone();
                            let core = Arc::clone(&core);
                            let command = *command;
                            tokio::spawn(async move {
                                let result = api.set_light(&id, &body).await;
                                if let Err(error) = &result {
                                    warn!("Hue : commande refusée pour {entity_id} : {error}");
                                }
                                core.command_done(command, // Le détail (qui peut citer l'adresse du pont) reste dans les logs.
                                    result.map_err(|_| "le pont Hue n'a pas accepté l'ordre".to_owned()));
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
