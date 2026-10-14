//! La boucle principale : relevé régulier des appareils et de la connexion,
//! publication des états, signalement des nouveaux appareils.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::{Background, CancellationToken, Core};
use ha_storage::{AuditRecord, Store};
use tracing::{info, warn};

use crate::error::FreeboxError;
use crate::pairing::{self, Pairing};
use crate::presence::Presence;

/// Fréquence des relevés.
const POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_mins(5);
/// Entité qui annonce le dernier appareil inconnu apparu sur le réseau.
const NEW_DEVICE_ENTITY: &str = "sensor.freebox_nouvel_appareil";

/// Poignée vers l'intégration Freebox qui tourne en arrière-plan.
#[derive(Debug)]
pub struct Freebox(Background);

impl Freebox {
    #[must_use]
    pub fn start(pairing: Pairing, store: Store, core: Arc<Core>) -> Self {
        Self::start_with_interval(pairing, store, core, POLL_INTERVAL)
    }

    /// Comme [`Freebox::start`], avec un autre intervalle de relevé (tests).
    #[must_use]
    pub fn start_with_interval(
        pairing: Pairing,
        store: Store,
        core: Arc<Core>,
        interval: Duration,
    ) -> Self {
        Self(Background::spawn("l'intégration Freebox", move |token| {
            run(pairing, store, core, token, interval)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(
    pairing: Pairing,
    store: Store,
    core: Arc<Core>,
    token: CancellationToken,
    interval: Duration,
) {
    info!(
        "Freebox : connexion à {} ({})",
        pairing.device_name, pairing.host
    );
    let mut backoff = interval;
    let mut session = None;
    let mut presence: Option<Presence> = None;

    loop {
        let result = async {
            if session.is_none() {
                let (opened, permissions) = pairing::connect(&pairing).await?;
                info!("Freebox : connectée (droits : {})", permissions.join(", "));
                session = Some(opened);
            }
            if presence.is_none() {
                presence = Some(Presence::new(
                    pairing::tracked(&store).await?,
                    pairing::known(&store).await?,
                ));
            }
            let (Some(session), Some(presence)) = (&session, &mut presence) else {
                return Err(FreeboxError::Unexpected("état interne".into()));
            };
            let hosts = session.hosts().await?;
            // La connexion Internet est un bonus : son échec n'empêche pas
            // de suivre la présence.
            let connection = session.connection().await.ok();
            let update = presence.update(&hosts, connection.as_ref(), Instant::now());
            for (entity_id, state, attributes) in update.states {
                if let Err(error) = core.states.set(entity_id, state, attributes) {
                    warn!("Freebox : état refusé ({error})");
                }
            }
            for (mac, name, vendor) in &update.new_devices {
                report_new_device(&store, &core, mac, name, vendor).await;
            }
            pairing::remember(&store, &update.to_remember).await
        }
        .await;

        let wait = match result {
            Ok(()) => {
                backoff = interval;
                interval
            }
            Err(error) => {
                warn!(
                    "Freebox : relevé impossible ({error}), nouvel essai dans {} s",
                    backoff.as_secs()
                );
                // On rouvrira une session au prochain essai.
                session = None;
                let wait = backoff;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                wait
            }
        };
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = token.cancelled() => break,
        }
    }
    info!("Freebox : arrêtée");
}

/// Un appareil jamais vu vient d'apparaître : journal d'audit, log, et
/// entité (pour une automatisation, par exemple).
async fn report_new_device(store: &Store, core: &Core, mac: &str, name: &str, vendor: &str) {
    warn!("Freebox : NOUVEL appareil sur le réseau : {name} ({mac}, {vendor})");
    let record = AuditRecord {
        actor: "freebox",
        action: "freebox_new_device",
        target: Some(mac),
        detail: Some(name),
        source: "réseau local",
    };
    if let Err(error) = store.audit(record).await {
        warn!("journal d'audit : écriture impossible ({error})");
    }
    let Ok(entity_id) = NEW_DEVICE_ENTITY.parse() else {
        return;
    };
    let mut attributes = ha_core::Attributes::new();
    attributes.insert("friendly_name".into(), "Nouvel appareil".into());
    attributes.insert("mac".into(), mac.into());
    attributes.insert("vendor".into(), vendor.into());
    if let Err(error) = core.states.set(entity_id, name, attributes) {
        warn!("Freebox : état refusé ({error})");
    }
}
