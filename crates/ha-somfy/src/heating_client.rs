//! La boucle du chauffage Atlantic : connexion au serveur Somfy avec le
//! compte (comme l'appli TaHoma), lecture des appareils, relevé des
//! événements, envoi des ordres, reconnexion automatique.
//!
//! Elle est à part de la boucle de la box ([`crate::Somfy`]) : chacune a sa
//! santé, et une panne du serveur Somfy ne touche pas aux volets.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Background, CancellationToken, Core, Event, Health};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

use crate::cloud::{CloudSession, SOMFY_EUROPE};
use crate::error::SomfyError;
use crate::heating::{self, Heating};

/// Relevé des événements : le chauffage change lentement.
const FETCH_INTERVAL: Duration = Duration::from_secs(30);
/// Après un ordre, relevé plus fréquent le temps que la pompe à chaleur réponde.
const FAST_FETCH: Duration = Duration::from_secs(5);
const FAST_WINDOW: Duration = Duration::from_secs(90);
/// Relecture complète de temps en temps, par sécurité.
const FULL_REFRESH: Duration = Duration::from_mins(30);
const FIRST_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_mins(5);
/// Mot de passe refusé : on attend longtemps, pour ne pas faire bloquer le
/// compte Somfy à force d'essais.
const BAD_CREDENTIALS_BACKOFF: Duration = Duration::from_hours(1);

/// Le compte Somfy (celui de l'appli TaHoma).
#[derive(Clone)]
pub struct CloudAccount {
    email: String,
    password: Zeroizing<String>,
}

impl CloudAccount {
    #[must_use]
    pub fn new(email: String, password: Zeroizing<String>) -> Self {
        Self { email, password }
    }
}

/// Jamais le mot de passe dans un affichage de débogage.
impl std::fmt::Debug for CloudAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudAccount")
            .field("password", &"<masqué>")
            .finish_non_exhaustive()
    }
}

/// Poignée vers la boucle du chauffage, en arrière-plan.
#[derive(Debug)]
pub struct HeatingCloud(Background);

impl HeatingCloud {
    #[must_use]
    pub fn start(account: CloudAccount, core: Arc<Core>) -> Self {
        let supervisor = Arc::clone(&core);
        // Le serveur Somfy dit s'il a accepté chaque ordre.
        core.answers_commands(heating::ID);
        Self(ha_core::supervise(
            supervisor,
            heating::ID,
            heating::NAME,
            move |token| run(account.clone(), Arc::clone(&core), token),
        ))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(account: CloudAccount, core: Arc<Core>, token: CancellationToken) {
    let http = match ha_tls::public_client() {
        Ok(http) => http,
        Err(error) => {
            error!("chauffage : impossible de préparer la connexion : {error}");
            return;
        }
    };
    let mut heating = Heating::default();
    let mut events = core.bus.subscribe();
    let mut backoff = FIRST_BACKOFF;
    info!("chauffage : connexion au serveur Somfy");

    'login: loop {
        // 1. Connexion avec le compte, 2. appareils, 3. écouteur d'événements.
        let connected = async {
            let session = CloudSession::login(
                http.clone(),
                SOMFY_EUROPE,
                &account.email,
                &account.password,
            )
            .await?;
            match start_session(&session, &mut heating, &core).await {
                Ok(listener) => Ok((session, listener)),
                Err(error) => {
                    session.logout().await;
                    Err(error)
                }
            }
        }
        .await;
        let (session, mut listener) = match connected {
            Ok(connected) => connected,
            Err(error) => {
                let (wait, message) = if matches!(error, SomfyError::BadCredentials) {
                    error!("chauffage : {error}, vérifie l'e-mail et le mot de passe Somfy");
                    (
                        BAD_CREDENTIALS_BACKOFF,
                        "compte Somfy refusé : vérifie l'e-mail et le mot de passe".to_owned(),
                    )
                } else {
                    warn!(
                        "chauffage : serveur Somfy injoignable ({error}), nouvel essai dans {} s",
                        backoff.as_secs()
                    );
                    (backoff, format!("serveur Somfy injoignable ({error})"))
                };
                core.report(
                    heating::ID,
                    heating::NAME,
                    Health::Retrying {
                        error: message,
                        retry_at: SystemTime::now() + wait,
                    },
                );
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = token.cancelled() => break 'login,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        info!("chauffage : connecté, {} entités", heating.len());
        core.report(heating::ID, heating::NAME, Health::Connected);
        backoff = FIRST_BACKOFF;
        let mut fast_until: Option<Instant> = None;
        let fetch = tokio::time::sleep(FETCH_INTERVAL);
        tokio::pin!(fetch);
        let refresh = tokio::time::sleep(FULL_REFRESH);
        tokio::pin!(refresh);

        loop {
            tokio::select! {
                () = token.cancelled() => {
                    session.logout().await;
                    break 'login;
                }

                () = &mut refresh => {
                    // Relecture complète avec la même session.
                    match start_session(&session, &mut heating, &core).await {
                        Ok(new_listener) => listener = new_listener,
                        Err(error) => {
                            warn!("chauffage : relecture impossible ({error}), reconnexion");
                            session.logout().await;
                            continue 'login;
                        }
                    }
                    refresh.as_mut().reset(Instant::now() + FULL_REFRESH);
                },

                () = &mut fetch => {
                    match session.fetch_events(&listener).await {
                        Ok(batch) => {
                            if heating.apply_events(&batch, &core.states) {
                                info!("chauffage : appareils ajoutés ou retirés, relecture");
                                refresh.as_mut().reset(Instant::now());
                            }
                        }
                        Err(error) => {
                            // Session ou écouteur expiré : on se reconnecte.
                            warn!("chauffage : relevé impossible ({error}), reconnexion");
                            session.logout().await;
                            continue 'login;
                        }
                    }
                    let now = Instant::now();
                    fetch.as_mut().reset(now + fetch_delay(now, fast_until));
                },

                received = events.recv() => match received {
                    Ok(event) => {
                        if let Event::CallService { domain, service, entity_id, data, command: order, issued_at, .. } = event.as_ref()
                            && let Some(command) = heating.command(domain, service, entity_id, data)
                        {
                            if ha_core::commands::is_stale(*issued_at) {
                                warn!("chauffage : ordre périmé ignoré pour {entity_id}");
                                core.command_done(*order, Err("ordre périmé : le serveur Somfy a mis trop longtemps".into()));
                                continue;
                            }
                            let now = Instant::now();
                            fast_until = Some(now + FAST_WINDOW);
                            fetch.as_mut().reset(now + FAST_FETCH);
                            let session = session.clone();
                            let entity_id = entity_id.clone();
                            let core = Arc::clone(&core);
                            let order = *order;
                            tokio::spawn(async move {
                                let names: Vec<&str> = command.commands.iter().map(|(name, _)| name.as_str()).collect();
                                debug!("chauffage : {} sur {entity_id}", names.join(", "));
                                let result = session.execute(&command.device_url, &command.commands).await;
                                if let Err(error) = &result {
                                    warn!("chauffage : ordre refusé pour {entity_id} : {error}");
                                }
                                core.command_done(order,
                                    result.map_err(|_| "le serveur Somfy n'a pas accepté l'ordre".to_owned()));
                            });
                        }
                    }
                    Err(RecvError::Lagged(missed)) => warn!("chauffage : {missed} événements manqués"),
                    Err(RecvError::Closed) => {
                        session.logout().await;
                        break 'login;
                    }
                },
            }
        }
    }
    info!("chauffage : arrêté");
}

/// Lit tous les appareils puis ouvre un écouteur d'événements.
async fn start_session(
    session: &CloudSession,
    heating: &mut Heating,
    core: &Core,
) -> Result<String, SomfyError> {
    heating.sync(session.devices().await?, &core.states);
    if heating.is_empty() {
        warn!("chauffage : aucun appareil Atlantic sur le compte Somfy");
    }
    session.register_listener().await
}

fn fetch_delay(now: Instant, fast_until: Option<Instant>) -> Duration {
    if fast_until.is_some_and(|until| now < until) {
        FAST_FETCH
    } else {
        FETCH_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_never_shows() {
        let account = CloudAccount::new(
            "moi@exemple.fr".into(),
            Zeroizing::new("secret-de-test".into()),
        );
        let shown = format!("{account:?}");
        assert!(!shown.contains("secret-de-test"));
        assert!(!shown.contains("moi@exemple.fr"));
    }

    #[test]
    fn faster_right_after_an_order() {
        let now = Instant::now();
        assert_eq!(fetch_delay(now, None), FETCH_INTERVAL);
        assert_eq!(fetch_delay(now, Some(now + FAST_WINDOW)), FAST_FETCH);
        assert_eq!(
            fetch_delay(now + FAST_WINDOW, Some(now + FAST_WINDOW)),
            FETCH_INTERVAL
        );
    }
}
