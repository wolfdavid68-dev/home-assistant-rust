//! Le serveur domotique : noyau, stockage SQLite, API HTTP/WebSocket et
//! appareils MQTT, Philips Hue et Somfy TaHoma. Sans MQTT configuré, quelques appareils simulés servent
//! de démonstration.
//!
//! Utilisation :
//!
//! ```text
//! ha-server                       démarre le serveur (Ctrl+C pour arrêter)
//! ha-server user add <nom>        crée un compte (le mot de passe est demandé)
//! ha-server user passwd <nom>     change un mot de passe (déconnecte partout)
//! ha-server user delete <nom>     supprime un compte
//! ha-server user list             liste les comptes
//! ha-server hue pair <adresse>    associe le pont Philips Hue
//! ha-server hue status            affiche le pont associé
//! ha-server hue forget            oublie le pont
//! ha-server somfy pair <adresse>  associe la box Somfy TaHoma
//! ha-server somfy devices         liste les appareils de la box
//! ha-server automations check     vérifie le fichier d'automatisations
//! ha-server healthcheck           vérifie qu'un serveur répond (pour Docker)
//! ```
//!
//! Variables d'environnement :
//! - `HA_DB_PATH` : fichier de la base (par défaut `ha.db`) ;
//! - `HA_KEY_PATH` : fichier de la clé qui chiffre les secrets de la base
//!   (par défaut, à côté de la base : `ha.key`) ;
//! - `HA_BIND` : adresse d'écoute de l'API (par défaut `127.0.0.1:8123`) ;
//! - `HA_HISTORY_DAYS` : nombre de jours d'historique conservés (par défaut 10) ;
//! - `HA_MQTT_HOST` : adresse du broker MQTT (sans elle : appareils simulés) ;
//! - `HA_MQTT_PORT` : port du broker (par défaut 1883) ;
//! - `HA_MQTT_USER` et `HA_MQTT_PASSWORD_FILE` (fichier contenant le mot de
//!   passe, conseillé) ou `HA_MQTT_PASSWORD` : identifiants du broker ;
//! - `HA_MQTT_DISCOVERY_PREFIX` : préfixe de découverte (par défaut `homeassistant`) ;
//! - `HA_AUTOMATIONS_PATH` : fichier d'automatisations (par défaut `automations.yaml`) ;
//! - `RUST_LOG` : niveau de détail des logs (par défaut `info`).

mod cli;
mod health;

use std::error::Error;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_api::ApiState;
use ha_auth::Auth;
use ha_automation::Engine;
use ha_core::{Attributes, Core, EntityId, Event};
use ha_hue::Hue;
use ha_mqtt::{Mqtt, MqttConfig};
use ha_somfy::Somfy;
use ha_storage::{Recorder, Store};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zeroize::Zeroizing;

const DEFAULT_DB_PATH: &str = "ha.db";
const DEFAULT_BIND: &str = "127.0.0.1:8123";
const DEFAULT_HISTORY_DAYS: u32 = 10;
const DEFAULT_AUTOMATIONS_PATH: &str = "automations.yaml";

/// Concept Rust : `#[tokio::main]` démarre le moteur asynchrone de tokio.
/// Une fonction `async` peut attendre (`.await`) sans bloquer le fil
/// d'exécution : pendant qu'une tâche attend, les autres avancent.
#[tokio::main]
async fn main() -> ExitCode {
    // Logs : niveau `info` par défaut, modifiable avec la variable RUST_LOG.
    // Couleurs seulement dans un vrai terminal : dans les logs de Docker,
    // elles deviendraient des codes illisibles.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_ansi(std::io::stdout().is_terminal())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        [] => run_server().await,
        ["user", rest @ ..] => cli::user_command(rest).await,
        ["hue", rest @ ..] => cli::hue_command(rest).await,
        ["somfy", rest @ ..] => cli::somfy_command(rest).await,
        ["automations", "check"] => cli::check_automations(&automations_path()),
        ["healthcheck"] => match parse_bind() {
            Ok(bind) => health::check(bind).await,
            Err(error) => Err(error.into()),
        },
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

fn automations_path() -> std::path::PathBuf {
    std::env::var("HA_AUTOMATIONS_PATH")
        .unwrap_or_else(|_| DEFAULT_AUTOMATIONS_PATH.to_owned())
        .into()
}

/// Ouvre la base, avec la clé qui chiffre les secrets (jetons Hue, Somfy).
async fn open_store() -> Result<Store, ha_storage::StorageError> {
    let db_path = std::path::PathBuf::from(db_path());
    let key_path = std::env::var_os("HA_KEY_PATH")
        .map_or_else(|| db_path.with_extension("key"), std::path::PathBuf::from);
    Store::open_with_key(&db_path, &key_path).await
}

fn db_path() -> String {
    std::env::var("HA_DB_PATH").unwrap_or_else(|_| DEFAULT_DB_PATH.to_owned())
}

async fn run_server() -> Result<(), Box<dyn Error>> {
    info!("ha-server {} (Ctrl+C pour arrêter)", ha_core::VERSION);

    let db_path = db_path();
    let history_days = history_days()?;
    let bind = bind_address()?;
    let mqtt_config = mqtt_config()?;
    // Un fichier d'automatisations invalide empêche le démarrage, avec un
    // message clair : mieux vaut corriger que de tourner à moitié.
    let automations = ha_automation::load(&automations_path())?;
    let store = open_store().await?;
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

    // Vrais appareils (MQTT) ou, à défaut, appareils simulés.
    let mqtt = mqtt_config.map(|config| Mqtt::start(config, Arc::clone(&core)));
    let engine = Engine::start(automations, Arc::clone(&core));
    // Une association illisible (clé de chiffrement perdue, par exemple)
    // ne doit pas empêcher le reste de la maison de fonctionner.
    let hue = match ha_hue::pairing::load(&store).await {
        Ok(pairing) => pairing.map(|pairing| Hue::start(pairing, Arc::clone(&core))),
        Err(error) => {
            warn!("Hue : association illisible ({error}). Refais « ha-server hue pair ».");
            None
        }
    };
    let somfy = match ha_somfy::pairing::load(&store).await {
        Ok(pairing) => pairing.map(|pairing| Somfy::start(pairing, Arc::clone(&core))),
        Err(error) => {
            warn!("Somfy : association illisible ({error}). Refais « ha-server somfy pair ».");
            None
        }
    };
    let mut simulation = if mqtt.is_none() && hue.is_none() && somfy.is_none() {
        info!("aucun appareil configuré (MQTT, Hue, Somfy) : appareils simulés");
        Some(tokio::spawn(simulate_devices(Arc::clone(&core))))
    } else {
        None
    };

    // Concept Rust : `select!` attend la première de plusieurs opérations.
    // Sans simulation, la deuxième branche attend indéfiniment
    // (`std::future::pending`) : seul Ctrl+C peut alors arrêter le serveur.
    tokio::select! {
        result = shutdown_signal() => result?,
        result = async {
            match &mut simulation {
                Some(task) => task.await,
                None => std::future::pending().await,
            }
        } => result??,
    }
    // Plus aucun changement d'état après ce point.
    if let Some(simulation) = simulation {
        simulation.abort();
    }
    engine.stop().await;
    if let Some(somfy) = somfy {
        somfy.stop().await;
    }
    if let Some(hue) = hue {
        hue.stop().await;
    }
    if let Some(mqtt) = mqtt {
        mqtt.stop().await;
    }
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

/// Lit la configuration MQTT. `None` si `HA_MQTT_HOST` n'est pas défini.
fn mqtt_config() -> Result<Option<MqttConfig>, String> {
    let Ok(host) = std::env::var("HA_MQTT_HOST") else {
        return Ok(None);
    };
    if host.is_empty() || host.len() > 253 || host.contains(char::is_whitespace) {
        return Err(format!("HA_MQTT_HOST invalide : {host:?}"));
    }
    let port = match std::env::var("HA_MQTT_PORT") {
        Err(_) => 1883,
        Ok(value) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| format!("HA_MQTT_PORT invalide : {value:?}"))?,
    };
    let username = std::env::var("HA_MQTT_USER").ok();
    // Un fichier est préférable à une variable d'environnement : celles-ci
    // sont visibles par les autres programmes du même utilisateur.
    let password = match std::env::var("HA_MQTT_PASSWORD_FILE") {
        Ok(path) => {
            let content =
                Zeroizing::new(std::fs::read_to_string(&path).map_err(|error| {
                    format!("lecture de HA_MQTT_PASSWORD_FILE ({path}) : {error}")
                })?);
            Some(Zeroizing::new(
                content.trim_end_matches(['\r', '\n']).to_owned(),
            ))
        }
        Err(_) => std::env::var("HA_MQTT_PASSWORD").ok().map(Zeroizing::new),
    };
    if password.is_some() && username.is_none() {
        return Err("mot de passe MQTT donné sans HA_MQTT_USER".into());
    }
    let discovery_prefix =
        std::env::var("HA_MQTT_DISCOVERY_PREFIX").unwrap_or_else(|_| "homeassistant".into());
    let prefix_ok = !discovery_prefix.is_empty()
        && !discovery_prefix.contains(['+', '#', '\0'])
        && !discovery_prefix.starts_with('/')
        && !discovery_prefix.ends_with('/');
    if !prefix_ok {
        return Err(format!(
            "HA_MQTT_DISCOVERY_PREFIX invalide : {discovery_prefix:?}"
        ));
    }

    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1");
    if !local {
        warn!("MQTT vers {host} : la connexion n'est PAS chiffrée (TLS pas encore disponible).");
        warn!("n'utilise qu'un broker sur ton réseau local de confiance, jamais via Internet.");
    }
    if username.is_none() {
        warn!("MQTT sans identifiants : n'importe qui sur le réseau peut piloter tes appareils.");
    }
    Ok(Some(MqttConfig {
        host,
        port,
        username,
        password,
        client_id: "ha-rust-server".into(),
        discovery_prefix,
    }))
}

/// Attend une demande d'arrêt : Ctrl+C, ou (Linux, macOS) le signal
/// `SIGTERM` que Docker et systemd envoient pour arrêter un programme.
///
/// Dans un conteneur, le programme est le « processus n° 1 », que le noyau
/// ne tue pas d'office : sans ce code, `SIGTERM` serait ignoré et Docker
/// finirait par le tuer brutalement (`SIGKILL`), sans enregistrer les
/// derniers états.
///
/// Concept Rust : `#[cfg(unix)]` ne compile un bloc que sur les systèmes
/// Unix. Windows n'a pas de `SIGTERM` : seul Ctrl+C y est attendu.
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => {
                info!("signal d'arrêt reçu (SIGTERM)");
                Ok(())
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

/// Lit `HA_BIND`, sans rien afficher.
fn parse_bind() -> Result<SocketAddr, String> {
    let value = std::env::var("HA_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    value
        .parse()
        .map_err(|_| format!("HA_BIND invalide : {value:?} (exemple : 127.0.0.1:8123)"))
}

/// Lit `HA_BIND`. Par défaut, l'API n'écoute que sur la machine elle-même.
fn bind_address() -> Result<SocketAddr, String> {
    let addr = parse_bind()?;
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
                Event::CallService {
                    domain,
                    service,
                    entity_id,
                    ..
                } => info!("service {domain}.{service} appelé pour {entity_id}"),
            },
            Err(RecvError::Lagged(missed)) => {
                warn!("{missed} événements manqués (abonné trop lent)");
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
