//! La boucle de SmartThings : renouveler les jetons à temps, lire les
//! appareils, relever leur état, et envoyer les ordres.
//!
//! SmartThings ne prévient pas la maison d'un changement (il faudrait une
//! adresse joignable depuis Internet) : l'état de chaque appareil est relu
//! toutes les [`POLL`], et quelques secondes après chaque ordre.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Background, CancellationToken, Core, Event, Health};
use ha_storage::{SmartThingsAccount, SmartThingsTokens, Store};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::SmartThingsError;
use crate::api::{Api, App, Endpoint};
use crate::devices::Devices;

/// L'état de chaque appareil est relu à ce rythme.
pub const POLL: Duration = Duration::from_secs(30);
/// La liste des appareils (et les jetons) sont revus à ce rythme.
const FULL_REFRESH: Duration = Duration::from_mins(10);
/// Les jetons sont renouvelés quand il leur reste moins que ça (ils vivent
/// 24 heures).
const RENEW_BEFORE: Duration = Duration::from_hours(6);
/// Après un ordre, l'appareil est relu après ces délais.
const AFTER_ORDER: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(10)];
const MAX_BACKOFF: Duration = Duration::from_mins(5);
const UNAUTHORIZED_WAIT: Duration = Duration::from_mins(30);

/// L'intégration SmartThings, en marche.
#[derive(Debug)]
pub struct SmartThings(Background);

impl SmartThings {
    /// Démarre (la maison doit être associée : `ha-server smartthings pair`).
    #[must_use]
    pub fn start(store: Store, endpoint: Endpoint, core: Arc<Core>) -> Self {
        let supervisor = Arc::clone(&core);
        core.answers_commands(crate::ID);
        Self(ha_core::supervise(
            supervisor,
            crate::ID,
            crate::NAME,
            move |token| run(store.clone(), endpoint.clone(), Arc::clone(&core), token),
        ))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

fn app(account: &SmartThingsAccount) -> App {
    App {
        client_id: account.client_id.clone(),
        client_secret: account.client_secret.clone(),
        redirect_uri: account.redirect_uri.clone(),
    }
}

/// Des jetons valides encore au moins [`RENEW_BEFORE`] : ceux de la base,
/// ou de nouveaux, enregistrés **avant** de s'en servir (l'ancien jeton de
/// renouvellement ne marche plus une fois utilisé).
pub async fn fresh_tokens(
    store: &Store,
    endpoint: &Endpoint,
    http: &reqwest::Client,
) -> Result<SmartThingsTokens, SmartThingsError> {
    let account = store
        .smartthings_account()
        .await?
        .ok_or(SmartThingsError::Unauthorized)?;
    let left = account
        .tokens
        .expires_at
        .duration_since(SystemTime::now())
        .unwrap_or_default();
    if left > RENEW_BEFORE {
        return Ok(account.tokens);
    }
    let tokens = app(&account)
        .refresh(http, endpoint, &account.tokens.refresh_token)
        .await?;
    if !store.update_smartthings_tokens(&tokens).await? {
        return Err(SmartThingsError::Unauthorized);
    }
    info!("SmartThings : jetons renouvelés");
    Ok(tokens)
}

async fn run(store: Store, endpoint: Endpoint, core: Arc<Core>, token: CancellationToken) {
    let http = match endpoint.http() {
        Ok(http) => http,
        Err(error) => {
            error!("SmartThings : impossible de préparer la connexion : {error}");
            return;
        }
    };
    let mut devices = Devices::default();
    let mut events = core.bus.subscribe();
    let (refresh_tx, mut refresh_rx) = mpsc::channel::<String>(64);
    let mut backoff = Duration::from_secs(5);
    let mut rooms: HashMap<String, String> = HashMap::new();

    'connection: loop {
        let connected = async {
            let tokens = fresh_tokens(&store, &endpoint, &http).await?;
            let api = Api::new(http.clone(), endpoint.clone(), tokens.access_token);
            let list = api.devices().await?;
            let mut locations: Vec<&String> =
                list.iter().filter_map(|d| d.location_id.as_ref()).collect();
            locations.sort();
            locations.dedup();
            for location in locations.into_iter().take(5) {
                match api.rooms(location).await {
                    Ok(found) => rooms.extend(found),
                    Err(error) => debug!("SmartThings : pièces illisibles ({error})"),
                }
            }
            devices.sync(list, &rooms, &core.states);
            Ok::<Api, SmartThingsError>(api)
        }
        .await;
        let api = match connected {
            Ok(api) => api,
            Err(error) => {
                let (wait, message) = match &error {
                    SmartThingsError::Unauthorized => (
                        UNAUTHORIZED_WAIT,
                        "accès refusé : refais « ha-server smartthings pair »".to_owned(),
                    ),
                    SmartThingsError::RateLimited(seconds) => {
                        (Duration::from_secs(*seconds), error.to_string())
                    }
                    _ => (backoff, format!("SmartThings injoignable ({error})")),
                };
                warn!(
                    "SmartThings : {message}, nouvel essai dans {} s",
                    wait.as_secs()
                );
                core.report(
                    crate::ID,
                    crate::NAME,
                    Health::Retrying {
                        error: message,
                        retry_at: SystemTime::now() + wait,
                    },
                );
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = token.cancelled() => break 'connection,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        info!("SmartThings : connecté, {} appareils", devices.len());
        core.report(crate::ID, crate::NAME, Health::Connected);
        backoff = Duration::from_secs(5);
        let poll = tokio::time::sleep(Duration::ZERO);
        tokio::pin!(poll);
        let refresh = tokio::time::sleep(FULL_REFRESH);
        tokio::pin!(refresh);

        loop {
            tokio::select! {
                () = token.cancelled() => break 'connection,

                () = &mut refresh => continue 'connection,

                () = &mut poll => {
                    let mut next = POLL;
                    for device_id in devices.ids() {
                        match read_status(&api, &mut devices, &device_id, &core).await {
                            Ok(()) => {}
                            Err(SmartThingsError::RateLimited(seconds)) => {
                                warn!("SmartThings : trop de demandes, pause de {seconds} s");
                                next = Duration::from_secs(seconds);
                                break;
                            }
                            Err(other) => {
                                warn!("SmartThings : relevé impossible ({other}), reconnexion");
                                continue 'connection;
                            }
                        }
                    }
                    poll.as_mut().reset(tokio::time::Instant::now() + next);
                },

                Some(device_id) = refresh_rx.recv() => {
                    if let Err(error) = read_status(&api, &mut devices, &device_id, &core).await {
                        debug!("SmartThings : relecture après l'ordre impossible ({error})");
                    }
                },

                received = events.recv() => match received {
                    Ok(event) => {
                        if let Event::CallService { domain, service, entity_id, data, command: order, issued_at, .. } = event.as_ref()
                            && devices.device_of(entity_id).is_some()
                        {
                            let Some(commands) = devices.command(domain, service, entity_id, data) else {
                                core.command_done(*order, Err("SmartThings ne sait pas faire ça avec cet appareil".into()));
                                continue;
                            };
                            if ha_core::commands::is_stale(*issued_at) {
                                warn!("SmartThings : ordre périmé ignoré pour {entity_id}");
                                core.command_done(*order, Err("ordre périmé".into()));
                                continue;
                            }
                            let api = api.clone();
                            let core = Arc::clone(&core);
                            let order = *order;
                            let entity_id = entity_id.clone();
                            let refresh_tx = refresh_tx.clone();
                            tokio::spawn(async move {
                                let result = api.command(&commands.device_id, &commands.commands).await;
                                if let Err(error) = &result {
                                    warn!("SmartThings : ordre refusé pour {entity_id} : {error}");
                                }
                                core.command_done(order, result.map_err(|_| "SmartThings n'a pas accepté l'ordre".to_owned()));
                                for wait in AFTER_ORDER {
                                    tokio::time::sleep(wait).await;
                                    let _ = refresh_tx.send(commands.device_id.clone()).await;
                                }
                            });
                        }
                    }
                    Err(RecvError::Lagged(missed)) => warn!("SmartThings : {missed} événements manqués"),
                    Err(RecvError::Closed) => break 'connection,
                },
            }
        }
    }
    info!("SmartThings : arrêté");
}

/// Relit l'état d'un appareil. Un appareil retiré ou interdit devient
/// « indisponible » sans arrêter les autres.
async fn read_status(
    api: &Api,
    devices: &mut Devices,
    device_id: &str,
    core: &Core,
) -> Result<(), SmartThingsError> {
    match api.status(device_id).await {
        Ok(status) => {
            devices.apply_status(device_id, status, &core.states);
            Ok(())
        }
        Err(SmartThingsError::Status(403 | 404 | 409 | 422)) => {
            devices.set_unavailable(device_id, &core.states);
            Ok(())
        }
        Err(error) => Err(error),
    }
}
