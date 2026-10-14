//! Démonstration de l'étape 1 : quelques entités simulées qui changent d'état,
//! et un abonné qui affiche chaque changement reçu sur le bus.
//!
//! Lancer avec `cargo run -p ha-server`, arrêter avec Ctrl+C.

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use ha_core::{Attributes, Core, EntityId, Event};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// Concept Rust : `#[tokio::main]` démarre le moteur asynchrone de tokio.
/// Une fonction `async` peut attendre (`.await`) sans bloquer le fil
/// d'exécution : pendant qu'une tâche attend, les autres avancent.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Logs : niveau `info` par défaut, modifiable avec la variable RUST_LOG.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    info!(
        "ha-server {} — démo de l'étape 1 (Ctrl+C pour arrêter)",
        ha_core::VERSION
    );

    let core = Arc::new(Core::new());

    // On s'abonne AVANT de créer les entités, pour ne rater aucun événement.
    let logger = tokio::spawn(log_state_changes(core.bus.subscribe()));
    let simulation = tokio::spawn(simulate_devices(Arc::clone(&core)));

    // Concept Rust : `select!` attend la première de plusieurs opérations.
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        result = simulation => result??,
    }

    info!("arrêt — états finaux :");
    for state in core.states.all() {
        info!("  {} = {}", state.entity_id(), state.state());
    }
    logger.abort();
    Ok(())
}

/// Affiche chaque changement d'état publié sur le bus.
async fn log_state_changes(mut rx: tokio::sync::broadcast::Receiver<Arc<Event>>) {
    loop {
        match rx.recv().await {
            Ok(event) => match event.as_ref() {
                Event::StateChanged {
                    entity_id,
                    old_state,
                    new_state,
                } => {
                    let old = old_state.as_ref().map_or("(nouveau)", |s| s.state());
                    let new = new_state.as_ref().map_or("(supprimé)", |s| s.state());
                    if let (true, Some(state)) = (old == new, new_state) {
                        // Seuls les attributs ont changé.
                        let attributes = serde_json::Value::Object(state.attributes().clone());
                        info!("{entity_id}: {new}, attributs {attributes}");
                    } else {
                        info!("{entity_id}: {old} -> {new}");
                    }
                }
            },
            Err(RecvError::Lagged(missed)) => {
                warn!("{missed} événements manqués (abonné trop lent)")
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// Simule quelques appareils. Dans les prochaines étapes, ils seront remplacés
/// par de vrais appareils (MQTT…).
async fn simulate_devices(core: Arc<Core>) -> Result<(), ha_core::CoreError> {
    let light: EntityId = "light.salon".parse()?;
    let temperature: EntityId = "sensor.temperature_salon".parse()?;
    let door: EntityId = "binary_sensor.porte_entree".parse()?;
    let plug: EntityId = "switch.prise_cafe".parse()?;

    core.states.set(plug.clone(), "off", Attributes::new())?;

    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    for tick in 0u32.. {
        ticker.tick().await;

        // Température qui oscille doucement autour de 21 °C.
        let celsius = 21.0 + 1.5 * (f64::from(tick) * 0.4).sin();
        core.states.set(
            temperature.clone(),
            format!("{celsius:.1}"),
            attributes(json!({"unit_of_measurement": "°C", "device_class": "temperature"})),
        )?;

        // La lumière s'allume et s'éteint toutes les 4 secondes ;
        // allumée, sa luminosité change à chaque seconde (attribut seul).
        let on = (tick / 4) % 2 == 0;
        let light_attributes = if on {
            attributes(json!({"brightness": 100 + (tick % 4) * 50}))
        } else {
            Attributes::new()
        };
        core.states.set(
            light.clone(),
            if on { "on" } else { "off" },
            light_attributes,
        )?;

        // La porte s'ouvre une seconde toutes les 6 secondes.
        let open = tick % 6 == 5;
        core.states.set(
            door.clone(),
            if open { "on" } else { "off" },
            attributes(json!({"device_class": "door"})),
        )?;
    }
    Ok(())
}

fn attributes(value: serde_json::Value) -> Attributes {
    match value {
        serde_json::Value::Object(map) => map,
        _ => Attributes::new(),
    }
}
