//! **Durée de conservation** : l'historique détaillé, le journal de bord et
//! le journal d'audit ne sont gardés que quelques jours (réglables). Les
//! lignes plus anciennes sont supprimées au démarrage, puis **chaque jour** :
//! sans cela, un serveur qui tourne des mois sans redémarrer verrait sa base
//! grossir sans limite.

use std::time::{Duration, SystemTime};

use ha_core::{Background, CancellationToken};
use tracing::{info, warn};

use crate::error::StorageError;
use crate::store::Store;

/// Le temps entre deux purges.
const PURGE_EVERY: Duration = Duration::from_hours(24);

/// Combien de jours garder chaque journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// L'historique des états, et le journal de bord avec lui.
    pub history_days: u32,
    /// Le journal d'audit.
    pub audit_days: u32,
}

/// Le nombre de lignes supprimées par une purge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Purged {
    pub history: u64,
    pub logbook: u64,
    pub audit: u64,
}

/// La date `days` jours avant `now`, jamais avant 1970.
///
/// Linux sait représenter une date d'avant 1970, Windows non : on s'arrête
/// à 1970 partout, pour que le résultat soit le même.
fn days_before(now: SystemTime, days: u32) -> SystemTime {
    now.checked_sub(Duration::from_hours(24 * u64::from(days)))
        .map_or(SystemTime::UNIX_EPOCH, |date| {
            date.max(SystemTime::UNIX_EPOCH)
        })
}

impl Store {
    /// Supprime ce qui a dépassé la durée de conservation, en prenant `now`
    /// comme date du jour.
    pub async fn purge_expired(
        &self,
        retention: Retention,
        now: SystemTime,
    ) -> Result<Purged, StorageError> {
        let history_limit = days_before(now, retention.history_days);
        Ok(Purged {
            history: self.purge_history(history_limit).await?,
            // Le journal de bord est gardé aussi longtemps que l'historique.
            logbook: self.purge_logbook(history_limit).await?,
            audit: self
                .purge_audit(days_before(now, retention.audit_days))
                .await?,
        })
    }

    /// Laisse SQLite mettre à jour ses statistiques sur les index (seulement
    /// là où c'est utile), pour qu'il continue de choisir les bons.
    /// Conseillé par SQLite pour un programme qui reste ouvert longtemps.
    pub async fn optimize(&self) -> Result<(), StorageError> {
        sqlx::query("PRAGMA optimize").execute(self.pool()).await?;
        Ok(())
    }
}

/// La tâche qui purge la base une fois par jour.
#[derive(Debug)]
pub struct Purge(Background);

impl Purge {
    /// Purge la base toutes les 24 heures. La première fois, 24 heures après
    /// l'appel : le démarrage vient en général de le faire.
    #[must_use]
    pub fn start(store: Store, retention: Retention) -> Self {
        Self(Background::spawn("la purge quotidienne", move |token| {
            run(store, retention, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(store: Store, retention: Retention, token: CancellationToken) {
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            () = tokio::time::sleep(PURGE_EVERY) => {}
        }
        purge_now(&store, retention).await;
    }
}

/// Une purge, suivie de `PRAGMA optimize`. Une erreur est signalée, sans
/// arrêter le serveur : la purge suivante réessaiera.
async fn purge_now(store: &Store, retention: Retention) {
    match store.purge_expired(retention, SystemTime::now()).await {
        Ok(purged) => info!(
            "purge quotidienne : {} ligne(s) d'historique, {} du journal de bord et {} du \
             journal d'audit supprimée(s)",
            purged.history, purged.logbook, purged.audit
        ),
        Err(error) => warn!("purge quotidienne impossible ({error})"),
    }
    if let Err(error) = store.optimize().await {
        warn!("optimisation de la base impossible ({error})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_before_counts_whole_days_and_never_underflows() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_hours(24 * 10);
        assert_eq!(
            days_before(now, 3),
            SystemTime::UNIX_EPOCH + Duration::from_hours(24 * 7)
        );
        assert_eq!(days_before(now, 0), now);
        assert_eq!(days_before(now, u32::MAX), SystemTime::UNIX_EPOCH);
    }
}
