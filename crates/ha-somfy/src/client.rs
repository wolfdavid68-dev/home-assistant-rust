//! La boucle principale : lecture des appareils, relevé des événements,
//! envoi des commandes, reconnexion automatique.

use std::sync::Arc;
use std::time::Duration;

use ha_core::{Background, CancellationToken, Core, Event};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::MissedTickBehavior;
use tracing::{debug, error, info, warn};

use crate::api::LocalApi;
use crate::devices::Devices;
use crate::error::SomfyError;
use crate::pairing::Pairing;

/// Fréquence de relevé des événements (la box ne les « pousse » pas).
const FETCH_INTERVAL: Duration = Duration::from_secs(2);
/// Relecture complète de temps en temps, par sécurité.
const FULL_REFRESH: Duration = Duration::from_mins(10);
const MAX_BACKOFF: Duration = Duration::from_mins(1);
/// Après un refus du jeton, on réessaie rarement : inutile d'insister.
const UNAUTHORIZED_BACKOFF: Duration = Duration::from_mins(15);

/// Poignée vers l'intégration Somfy qui tourne en arrière-plan.
#[derive(Debug)]
pub struct Somfy(Background);

impl Somfy {
    #[must_use]
    pub fn start(pairing: Pairing, core: Arc<Core>) -> Self {
        Self(Background::spawn("l'intégration Somfy", |token| {
            run(pairing, core, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(pairing: Pairing, core: Arc<Core>, token: CancellationToken) {
    let http = match ha_tls::client(Some(pairing.fingerprint)) {
        Ok((http, _)) => http,
        Err(error) => {
            error!("Somfy : impossible de préparer la connexion : {error}");
            return;
        }
    };
    let api = LocalApi::new(http, &pairing.host, pairing.token);
    let mut devices = Devices::default();
    let mut events = core.bus.subscribe();
    let mut backoff = Duration::from_secs(1);
    info!(
        "Somfy : connexion à la box {} ({})",
        pairing.gateway_id, pairing.host
    );

    'connection: loop {
        // 1. Liste complète des appareils, puis 2. écouteur d'événements.
        let connected = async {
            devices.sync(api.devices().await?, &core.states);
            api.register_listener().await
        }
        .await;
        let listener = match connected {
            Ok(listener) => listener,
            Err(error) => {
                let wait = if matches!(error, SomfyError::Unauthorized) {
                    error!("Somfy : {error}. Refais « ha-server somfy pair ».");
                    UNAUTHORIZED_BACKOFF
                } else {
                    warn!(
                        "Somfy : box injoignable ({error}), nouvel essai dans {} s",
                        backoff.as_secs()
                    );
                    backoff
                };
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = token.cancelled() => break 'connection,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        info!("Somfy : connecté, {} appareils", devices.len());
        backoff = Duration::from_secs(1);
        let mut fetch = tokio::time::interval(FETCH_INTERVAL);
        fetch.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let refresh = tokio::time::sleep(FULL_REFRESH);
        tokio::pin!(refresh);

        loop {
            tokio::select! {
                () = token.cancelled() => break 'connection,

                () = &mut refresh => continue 'connection,

                _ = fetch.tick() => match api.fetch_events(&listener).await {
                    Ok(batch) => {
                        if devices.apply_events(&batch, &core.states) {
                            info!("Somfy : appareils ajoutés ou retirés, relecture");
                            continue 'connection;
                        }
                    }
                    Err(error) => {
                        // Écouteur expiré, box redémarrée… : on recommence tout.
                        warn!("Somfy : relevé impossible ({error}), reconnexion");
                        continue 'connection;
                    }
                },

                received = events.recv() => match received {
                    Ok(event) => {
                        if let Event::CallService { domain, service, entity_id, data, .. } = event.as_ref()
                            && let Some(command) = devices.command(domain, service, entity_id, data)
                        {
                            let api = api.clone();
                            let entity_id = entity_id.clone();
                            tokio::spawn(async move {
                                debug!("Somfy : {} sur {entity_id}", command.name);
                                if let Err(error) = api
                                    .execute(&command.device_url, &command.name, command.parameters)
                                    .await
                                {
                                    warn!("Somfy : commande refusée pour {entity_id} : {error}");
                                }
                            });
                        }
                    }
                    Err(RecvError::Lagged(missed)) => warn!("Somfy : {missed} événements manqués"),
                    Err(RecvError::Closed) => break 'connection,
                },
            }
        }
    }
    info!("Somfy : arrêté");
}
