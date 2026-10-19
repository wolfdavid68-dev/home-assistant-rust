//! La page « Système » (étape 47) : ce que le serveur consomme, mesuré, et le
//! budget qu'il doit tenir.
//!
//! Inspiré de Moli OS, qui mesure chaque livraison contre Home Assistant :
//! une régression de mémoire ou de temps de réponse se voit tout de suite,
//! au lieu de s'installer en silence.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use ha_auth::Permission;
use serde_json::{Value, json};

use crate::ApiState;
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::roles;

/// Le budget : au-delà, la page le signale (et la CI refuse l'image).
pub const BUDGET_MEMORY_MIB: u64 = 64;
pub const BUDGET_CPU_PERCENT: f64 = 1.0;
pub const BUDGET_API_P95_MS: f64 = 25.0;
/// Les dernières requêtes dont on garde la durée.
const LATENCY_SAMPLES: usize = 500;
/// Tics d'horloge du noyau Linux par seconde (`CLK_TCK`, 100 partout).
const CLOCK_TICKS: f64 = 100.0;

/// Ce que l'API mesure en continu.
#[derive(Debug)]
pub(crate) struct Metrics {
    started: Instant,
    /// Durée des dernières requêtes `/api/…`, en microsecondes.
    latencies: Mutex<VecDeque<u32>>,
    /// Le relevé processeur précédent : (quand, tics consommés).
    cpu: Mutex<Option<(Instant, u64)>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            latencies: Mutex::new(VecDeque::with_capacity(LATENCY_SAMPLES)),
            cpu: Mutex::new(None),
        }
    }
}

impl Metrics {
    fn record(&self, took: Duration) {
        let mut latencies = self
            .latencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if latencies.len() == LATENCY_SAMPLES {
            latencies.pop_front();
        }
        latencies.push_back(u32::try_from(took.as_micros()).unwrap_or(u32::MAX));
    }

    /// Médiane et 95e centile, en millisecondes.
    fn percentiles(&self) -> Option<(f64, f64, usize)> {
        let mut sorted: Vec<u32> = self
            .latencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .copied()
            .collect();
        if sorted.is_empty() {
            return None;
        }
        sorted.sort_unstable();
        let at = |q: f64| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                reason = "rang dans une liste de 500"
            )]
            let index = ((sorted.len() - 1) as f64 * q).round() as usize;
            f64::from(sorted.get(index).copied().unwrap_or_default()) / 1000.0
        };
        Some((at(0.5), at(0.95), sorted.len()))
    }

    /// Le processeur utilisé depuis le relevé précédent (ou depuis le
    /// démarrage), en pourcentage d'un cœur.
    fn cpu_percent(&self, ticks: u64) -> f64 {
        let now = Instant::now();
        let mut previous = self.cpu.lock().unwrap_or_else(PoisonError::into_inner);
        let (since, before) = previous.unwrap_or((self.started, 0));
        *previous = Some((now, ticks));
        let seconds = now.duration_since(since).as_secs_f64();
        if seconds <= 0.0 {
            return 0.0;
        }
        #[expect(clippy::cast_precision_loss, reason = "des tics d'horloge")]
        let used = ticks.saturating_sub(before) as f64 / CLOCK_TICKS;
        (used / seconds * 100.0 * 100.0).round() / 100.0
    }
}

/// Mesure la durée des requêtes `/api/…` (sauf le WebSocket, qui dure).
pub(crate) async fn measure(
    State(state): State<Arc<ApiState>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let timed = path.starts_with("/api/") && path != "/api/websocket";
    let started = Instant::now();
    let response = next.run(request).await;
    if timed {
        state.metrics.record(started.elapsed());
    }
    response
}

/// La mémoire du processus et le processeur consommé, lus dans `/proc`
/// (Linux) : (mémoire résidente, mémoire anonyme, tics).
fn read_proc() -> Option<(u64, u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    };
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Le nom du programme est entre parenthèses ; les champs suivent.
    let after = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = after.split_whitespace().collect();
    // utime et stime sont les champs 14 et 15 (12 et 13 après le nom).
    let ticks = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    Some((field("VmRSS:")?, field("RssAnon:").unwrap_or(0), ticks))
}

/// `GET /api/system` : ce que le serveur consomme (administrateur).
pub(crate) async fn system(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadAudit)?;
    let process = read_proc();
    let cpu = process.map(|(_, _, ticks)| state.metrics.cpu_percent(ticks));
    let latency = state.metrics.percentiles();
    let database = state.store.database_size().await.ok();
    // La rapidité des appareils (étape 63), par intégration, sous son nom.
    let names: Vec<(&str, &str)> = state
        .core
        .integrations()
        .iter()
        .map(|status| (status.id, status.name))
        .collect();
    let devices: Vec<Value> = state
        .core
        .latencies
        .summaries()
        .into_iter()
        .map(|summary| {
            let name = names
                .iter()
                .find(|(id, _)| *id == summary.owner)
                .map_or(summary.owner.as_str(), |(_, name)| name);
            json!({
                "integration": summary.owner,
                "name": name,
                "p50_ms": summary.p50_ms,
                "p95_ms": summary.p95_ms,
                "samples": summary.samples,
                "slow": summary.slow,
            })
        })
        .collect();
    Ok(Json(json!({
        "version": ha_core::VERSION,
        "uptime_seconds": state.metrics.started.elapsed().as_secs(),
        "memory": process.map(|(resident, anonymous, _)| json!({"resident_bytes": resident, "anonymous_bytes": anonymous})),
        "cpu_percent": cpu,
        "api": latency.map(|(p50, p95, samples)| json!({"p50_ms": p50, "p95_ms": p95, "samples": samples})),
        "database_bytes": database,
        "devices": devices,
        "slow_ms": u64::try_from(ha_core::latency::SLOW.as_millis()).unwrap_or(u64::MAX),
        "entities": state.core.states.all().len(),
        "budget": {
            "memory_mib": BUDGET_MEMORY_MIB,
            "cpu_percent": BUDGET_CPU_PERCENT,
            "api_p95_ms": BUDGET_API_P95_MS,
        },
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp, reason = "tests")]

    use super::*;

    #[test]
    fn percentiles_of_the_last_requests() {
        let metrics = Metrics::default();
        assert!(metrics.percentiles().is_none());
        for ms in 1..=100 {
            metrics.record(Duration::from_millis(ms));
        }
        let (p50, p95, samples) = metrics.percentiles().unwrap();
        assert_eq!(samples, 100);
        assert!((50.0..=51.0).contains(&p50), "{p50}");
        assert!((95.0..=96.0).contains(&p95), "{p95}");
        // On ne garde que les dernières.
        for _ in 0..LATENCY_SAMPLES {
            metrics.record(Duration::from_millis(2));
        }
        assert_eq!(metrics.percentiles().unwrap(), (2.0, 2.0, LATENCY_SAMPLES));
    }

    #[test]
    fn the_process_reads_itself_on_linux() {
        if cfg!(target_os = "linux") {
            let (resident, _, _) = read_proc().unwrap();
            assert!(resident > 1024 * 1024, "{resident}");
        }
    }
}
