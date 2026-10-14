//! Le serveur domotique : noyau, stockage SQLite, API HTTP/WebSocket et, en
//! attendant les vrais appareils (étape 4), quelques entités simulées.
//!
//! Utilisation :
//!
//! ```text
//! ha-server                       démarre le serveur (Ctrl+C pour arrêter)
//! ha-server user add <nom>        crée un compte (le mot de passe est demandé)
//! ha-server user passwd <nom>     change un mot de passe (déconnecte partout)
//! ha-server user delete <nom>     supprime un compte
//! ha-server user list             liste les comptes
//! ```
//!
//! Variables d'environnement :
//! - `HA_DB_PATH` : fichier de la base (par défaut `ha.db`) ;
//! - `HA_BIND` : adresse d'écoute de l'API (par défaut `127.0.0.1:8123`) ;
//! - `HA_HISTORY_DAYS` : nombre de jours d'historique conservés (par défaut 10) ;
//! - `RUST_LOG` : niveau de détail des logs (par défaut `info`).

mod cli;

use std::error::Error;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_api::ApiState;
use ha_auth::Auth;
use ha_core::{Attributes, Core, EntityId, Event};
use ha_storage::{Recorder, Store};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const DEFAULT_DB_PATH: &str = "ha.db";
const DEFAULT_BIND: &str = "127.0.0.1:8123";
const DEFAULT_HISTORY_DAYS: u32 = 10;

/// Concept Rust : `#[tokio::main]` démarre le moteur asynchrone de tokio.
/// Une fonction `async` peut attendre (`.await`) sans bloquer le fil
/// d'exécution : pendant qu'une tâche attend, les autres avancent.
#[tokio::main]
async fn main() -> ExitCode {
    // Logs : niveau `info` par défaut, modifiable avec la variable RUST_LOG.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        [] => run_server().await,
        ["user", rest @ ..] => cli::user_command(rest, &db_path()).await,
        _ => Err(cli::USAGE.into()),
    };
    // On affiche le message lisible de l'erreur (`Display`), pas sa forme
    // technique (`Debug`) qu'afficherait un `main` qui renvoie `Result`.
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("erreur : {error}");
            ExitCode::FAILURE
        }
    }
}

fn db_path() -> String {
    std::env::var("HA_DB_PATH").unwrap_or_else(|_| DEFAULT_DB_PATH.to_owned())
}

async fn run_server() -> Result<(), Box<dyn Error>> {
    info!("ha-server {} (Ctrl+C pour arrêter)", ha_core::VERSION);

    let db_path = db_path();
    let history_days = history_days()?;
    let bind = bind_address()?;
    let store = Store::open(&db_path).await?;
    info!("base de données : {db_path}");

    // On oublie l'historique trop ancien.
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(history_days) * 24 * 3600))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let purged = store.purge_history(cutoff).await?;
    info!("historique conservé : {history_days} jours ({purged} lignes anciennes supprimées)");

    let auth = Arc::new(Auth::new(&store).await?);
    auth.purge_expired_tokens().await?;
    if auth.list_users().await?.is_empty() {
        warn!("aucun compte : l'API refusera toutes les connexions.");
        warn!("crée un compte avec : ha-server user add <nom>");
    }

    // On recharge les états d'avant l'arrêt, AVANT de démarrer quoi que ce soit.
    let core = Arc::new(Core::new());
    let restored = store.restore_into(&core.states).await?;
    info!("{restored} états restaurés depuis la base");
    for state in core.states.all() {
        info!("  {} = {} (restauré)", state.entity_id(), state.state());
    }

    let recorder = Recorder::start(store.clone(), Arc::clone(&core));

    // API HTTP / WebSocket.
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!("API sur http://{bind}/api/");
    let api_state = ApiState::new(Arc::clone(&core), store.clone(), auth);
    let api = tokio::spawn(ha_api::serve(listener, Arc::clone(&api_state)));

    // On s'abonne AVANT de créer les entités, pour ne rater aucun événement.
    let logger = tokio::spawn(log_state_changes(core.bus.subscribe()));
    let mut simulation = tokio::spawn(simulate_devices(Arc::clone(&core)));

    // Concept Rust : `select!` attend la première de plusieurs opérations.
    // `&mut simulation` emprunte la tâche au lieu de la consommer, pour pouvoir
    // l'arrêter ensuite.
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        result = &mut simulation => result??,
    }
    // Plus aucun changement d'état après ce point.
    simulation.abort();
    api_state.shutdown();
    if let Err(error) = api.await? {
        warn!("arrêt de l'API : {error}");
    }

    info!("arrêt — états finaux :");
    for state in core.states.all() {
        info!("  {} = {}", state.entity_id(), state.state());
    }
    logger.abort();
    // L'enregistreur écrit les derniers changements avant de fermer la base.
    recorder.stop().await;
    store.close().await;
    Ok(())
}

/// Lit `HA_HISTORY_DAYS`. Une valeur invalide arrête le programme avec un
/// message clair, plutôt que d'être ignorée en silence.
fn history_days() -> Result<u32, String> {
    match std::env::var("HA_HISTORY_DAYS") {
        Err(_) => Ok(DEFAULT_HISTORY_DAYS),
        Ok(value) => value
            .parse::<u32>()
            .ok()
            .filter(|days| (1..=3650).contains(days))
            .ok_or_else(|| format!("HA_HISTORY_DAYS invalide : {value:?} (entre 1 et 3650)")),
    }
}

/// Lit `HA_BIND`. Par défaut, l'API n'écoute que sur la machine elle-même.
fn bind_address() -> Result<SocketAddr, String> {
    let value = std::env::var("HA_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let addr: SocketAddr = value
        .parse()
        .map_err(|_| format!("HA_BIND invalide : {value:?} (exemple : 127.0.0.1:8123)"))?;
    if !addr.ip().is_loopback() {
        warn!("l'API écoute sur {addr} : elle est joignable depuis le réseau.");
        warn!("ne l'expose JAMAIS directement sur Internet : passe par un VPN.");
    }
    Ok(addr)
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
