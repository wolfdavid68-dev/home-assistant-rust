//! **Statistiques d'historique** (étape 59), comme `history_stats` dans Home
//! Assistant : combien de temps le chauffage a tourné aujourd'hui, combien
//! de fois la porte du garage s'est ouverte cette semaine, quelle part de la
//! journée la lumière du salon est restée allumée.
//!
//! ```yaml
//! history_stats:
//!   chauffage_du_jour:
//!     name: Chauffage aujourd'hui
//!     entity_id: switch.radiateur
//!     state: "on"          # ou une liste : [heat, on]
//!     type: time           # time (heures), count (nombre de fois), ratio (%)
//!     cycle: daily         # hourly, daily, weekly, monthly, yearly
//! ```
//!
//! Le calcul part de l'historique enregistré (il survit donc à un
//! redémarrage) et il est refait chaque minute.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId};
use ha_storage::Store;
use jiff::tz::TimeZone;
use serde_json::json;
use tracing::{info, warn};

use crate::compute::period_start;
use crate::config::{Derived, Kind, Measure};

/// La valeur affichée : des heures, un nombre de fois, ou une part de la
/// période en %.
#[must_use]
pub fn value(measure: Measure, inside: Duration, entries: u64, period: Duration) -> f64 {
    match measure {
        Measure::Time => (inside.as_secs_f64() / 3600.0 * 100.0).round() / 100.0,
        #[allow(
            clippy::cast_precision_loss,
            reason = "un nombre d'entrées, bien en deçà de 2^52"
        )]
        Measure::Count => entries as f64,
        Measure::Ratio if period.is_zero() => 0.0,
        Measure::Ratio => (inside.as_secs_f64() / period.as_secs_f64() * 1000.0).round() / 10.0,
    }
}

/// La tâche des statistiques d'historique.
#[derive(Debug)]
pub struct HistoryStats(Background);

impl HistoryStats {
    /// Démarre la tâche, s'il y a des statistiques dans `derived`.
    #[must_use]
    pub fn start(derived: &[Derived], store: Store, core: Arc<Core>) -> Option<Self> {
        let items: Vec<Derived> = derived
            .iter()
            .filter(|d| matches!(d.kind, Kind::HistoryStats { .. }))
            .cloned()
            .collect();
        if items.is_empty() {
            return None;
        }
        info!("{} statistique(s) d'historique", items.len());
        Some(Self(Background::spawn(
            "les statistiques d'historique",
            move |token| run(items, store, core, token),
        )))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(items: Vec<Derived>, store: Store, core: Arc<Core>, token: CancellationToken) {
    let mut ticker = tokio::time::interval(Duration::from_mins(1));
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {}
        }
        let zone = TimeZone::system();
        let now = SystemTime::now();
        let local = jiff::Zoned::now().with_time_zone(zone.clone()).datetime();
        for item in &items {
            let Kind::HistoryStats {
                source,
                states,
                measure: kind,
                cycle,
            } = &item.kind
            else {
                continue;
            };
            let start_local = period_start(*cycle, local);
            let Ok(start) = start_local
                .to_zoned(zone.clone())
                .map(|z| SystemTime::from(z.timestamp()))
            else {
                continue;
            };
            let (seconds, entries) = match store.time_in_states(source, states, start, now).await {
                Ok(found) => found,
                Err(error) => {
                    warn!("{} : historique illisible ({error})", item.entity_id);
                    continue;
                }
            };
            let period = now.duration_since(start).unwrap_or_default();
            let value = value(*kind, Duration::from_secs_f64(seconds), entries, period);
            publish(
                &core,
                item,
                source,
                states,
                *kind,
                &start_local.to_string(),
                value,
            );
        }
    }
}

fn publish(
    core: &Core,
    item: &Derived,
    source: &EntityId,
    states: &[String],
    kind: Measure,
    since: &str,
    value: f64,
) {
    let (unit, class) = match kind {
        Measure::Time => ("h", Some("duration")),
        Measure::Count => ("fois", None),
        Measure::Ratio => ("%", None),
    };
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!(item.name));
    attributes.insert("unit_of_measurement".into(), json!(unit));
    attributes.insert("state_class".into(), json!("measurement"));
    if let Some(class) = class {
        attributes.insert("device_class".into(), json!(class));
    }
    attributes.insert("source".into(), json!(source.as_str()));
    attributes.insert("etats".into(), json!(states));
    attributes.insert("depuis".into(), json!(since));
    let shown = if kind == Measure::Count {
        format!("{value:.0}")
    } else {
        value.to_string()
    };
    if let Err(error) = core.states.set(item.entity_id.clone(), shown, attributes) {
        warn!("{} : état refusé ({error})", item.entity_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_shown_in_hours_times_or_percent() {
        let hour = Duration::from_hours(1);
        let half_day = Duration::from_hours(12);
        assert!(
            (value(
                Measure::Time,
                hour * 2 + Duration::from_secs(18),
                0,
                half_day
            ) - 2.01)
                .abs()
                < 1e-9
        );
        assert!((value(Measure::Count, hour, 7, half_day) - 7.0).abs() < 1e-9);
        assert!((value(Measure::Ratio, hour * 4, 0, half_day) - 33.3).abs() < 1e-9);
        assert!(value(Measure::Ratio, hour, 0, Duration::ZERO).abs() < 1e-9);
    }
}
