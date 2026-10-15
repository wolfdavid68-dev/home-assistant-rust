//! Le serveur domotique : noyau, stockage SQLite, API HTTP/WebSocket et
//! appareils MQTT, Philips Hue et Somfy TaHoma. Sans MQTT configuré, quelques appareils simulés servent
//! de démonstration.
//!
//! Utilisation :
//!
//! ```text
//! ha-server                       démarre le serveur (Ctrl+C pour arrêter)
//! ha-server user add <nom> [rôle] crée un compte (admin, famille ou invité ;
//!                                 le mot de passe est demandé)
//! ha-server user role <nom> <rôle> change le rôle d'un compte
//! ha-server user passwd <nom>     change un mot de passe (déconnecte partout)
//! ha-server user delete <nom>     supprime un compte
//! ha-server user list             liste les comptes
//! ha-server user totp <nom>       active la double authentification
//! ha-server user totp-off <nom>   la désactive
//! ha-server hue pair <adresse>    associe le pont Philips Hue
//! ha-server hue status            affiche le pont associé
//! ha-server hue forget            oublie le pont
//! ha-server somfy pair <adresse>  associe la box Somfy TaHoma
//! ha-server somfy devices         liste les appareils de la box
//! ha-server freebox pair          associe la Freebox (lecture seule)
//! ha-server freebox devices       liste les appareils du réseau
//! ha-server freebox track <MAC> <nom>  suit la présence d'un appareil
//! ha-server automations check     vérifie automatisations, scripts et scènes
//! ha-server automations list      liste les automatisations (fichier et éditeur)
//! ha-server automations delete <id>  supprime une automatisation de l'éditeur
//! ha-server areas check           vérifie le fichier des pièces
//! ha-server alexa check           vérifie le fichier des appareils pour Alexa
//! ha-server thermostats check     vérifie le fichier des thermostats
//! ha-server persons check         vérifie le fichier des personnes
//! ha-server inputs check          vérifie le fichier des entrées
//! ha-server audit [nombre]        affiche le journal d'audit
//! ha-server audit verify          vérifie que le journal n'a pas été modifié
//! ha-server backup now          sauvegarde la base (même serveur en marche)
//! ha-server backup list          liste les sauvegardes
//! ha-server backup restore <f>   restaure une sauvegarde (serveur arrêté)
//! ha-server healthcheck           vérifie qu'un serveur répond (pour Docker)
//! ```
//!
//! Variables d'environnement :
//! - `HA_DB_PATH` : fichier de la base (par défaut `ha.db`) ;
//! - `HA_KEY_PATH` : fichier de la clé qui chiffre les secrets de la base
//!   (par défaut, à côté de la base : `ha.key`) ;
//! - `HA_BIND` : adresse d'écoute de l'API (par défaut `127.0.0.1:8123`) ;
//! - `HA_HISTORY_DAYS` : nombre de jours d'historique conservés (par défaut 10) ;
//! - `HA_AUDIT_DAYS` : nombre de jours de journal d'audit conservés (par défaut 365) ;
//! - `HA_MQTT_HOST` : adresse du broker MQTT (sans elle : appareils simulés) ;
//! - `HA_MQTT_PORT` : port du broker (par défaut 1883) ;
//! - `HA_MQTT_USER` et `HA_MQTT_PASSWORD_FILE` (fichier contenant le mot de
//!   passe, conseillé) ou `HA_MQTT_PASSWORD` : identifiants du broker ;
//! - `HA_MQTT_DISCOVERY_PREFIX` : préfixe de découverte (par défaut `homeassistant`) ;
//! - `HA_LATITUDE` et `HA_LONGITUDE` : position de la maison, pour le soleil
//!   (deux décimales suffisent, par exemple 48.85 et 2.35) ;
//! - `HA_PERSONS_PATH` : fichier des personnes (par défaut `persons.yaml`) ;
//! - `HA_INPUTS_PATH` : fichier des entrées (par défaut `inputs.yaml`) ;
//! - `HA_THERMOSTATS_PATH` : fichier des thermostats (par défaut `thermostats.yaml`) ;
//! - `HA_BACKUP_DIR`, `HA_BACKUP_TIME` (03:30), `HA_BACKUP_KEEP` (7) :
//!   sauvegardes automatiques chaque nuit ;
//! - `HA_METEO` : `on` pour la météo locale avec Open-Meteo (il faut la
//!   position de la maison) ; `HA_METEO_URL` pour un serveur auto-hébergé ;
//! - `HA_SCRIPTS_PATH` et `HA_SCENES_PATH` : scripts et scènes (par défaut
//!   `scripts.yaml` et `scenes.yaml`) ;
//! - `HA_NTFY_URL`, `HA_NTFY_TOPIC`, `HA_NTFY_TOKEN_FILE` : notifications sur
//!   le téléphone avec ntfy (entité `notify.telephone`) ;
//! - `HA_AREAS_PATH` : fichier des pièces (par défaut `areas.yaml`) ;
//! - `HA_AUTOMATIONS_PATH` : fichier d'automatisations (par défaut `automations.yaml`) ;
//! - `HA_ALEXA_IP` : adresse de cette machine sur le réseau local, pour
//!   piloter les appareils de `alexa.yaml` à la voix (`HA_ALEXA_PATH`,
//!   `HA_ALEXA_PORT` (80), `HA_ALEXA_ALLOWED` : adresses des Echo) ;
//! - `HA_AUTOMATION_EDITOR` : `on` (par défaut) ou `off` pour un éditeur
//!   d'automatisations en lecture seule dans le tableau de bord ;
//! - `RUST_LOG` : niveau de détail des logs (par défaut `info`).

mod backup;
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
use ha_freebox::Freebox;
use ha_hue::Hue;
use ha_mqtt::{Mqtt, MqttConfig};
use ha_somfy::Somfy;
use ha_storage::{Recorder, Store};
use ha_sun::{Location, SunTracker};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zeroize::Zeroizing;

const DEFAULT_DB_PATH: &str = "ha.db";
const DEFAULT_BIND: &str = "127.0.0.1:8123";
const DEFAULT_HISTORY_DAYS: u32 = 10;
const DEFAULT_AUDIT_DAYS: u32 = 365;
const DEFAULT_AUTOMATIONS_PATH: &str = "automations.yaml";
const DEFAULT_AREAS_PATH: &str = "areas.yaml";

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
        ["freebox", rest @ ..] => cli::freebox_command(rest).await,
        ["automations", rest @ ..] => cli::automations_command(rest).await,
        ["areas", "check"] => cli::check_areas(&areas_path()),
        ["alexa", "check"] => cli::check_alexa(&alexa_path()),
        ["thermostats", "check"] => cli::check_thermostats(&thermostats_path()),
        ["persons", "check"] => cli::check_persons(&persons_path()),
        ["inputs", "check"] => cli::check_inputs(&inputs_path()),
        ["audit", rest @ ..] => cli::audit_command(rest).await,
        ["backup", rest @ ..] => backup::command(rest).await,
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

pub(crate) fn scripts_path() -> std::path::PathBuf {
    std::env::var("HA_SCRIPTS_PATH")
        .unwrap_or_else(|_| "scripts.yaml".to_owned())
        .into()
}

pub(crate) fn scenes_path() -> std::path::PathBuf {
    std::env::var("HA_SCENES_PATH")
        .unwrap_or_else(|_| "scenes.yaml".to_owned())
        .into()
}

/// Lit un fichier de scripts ou de scènes ; l'erreur indique le fichier.
fn load_named<T>(
    path: &std::path::Path,
    parse: impl Fn(&str) -> Result<Vec<T>, ha_automation::AutomationError>,
) -> Result<Vec<T>, String> {
    ha_automation::scripts::load_file(path, parse)
        .map_err(|error| format!("{} : {error}", path.display()))
}

pub(crate) fn automations_path() -> std::path::PathBuf {
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

fn areas_path() -> std::path::PathBuf {
    std::env::var("HA_AREAS_PATH")
        .unwrap_or_else(|_| DEFAULT_AREAS_PATH.to_owned())
        .into()
}

pub(crate) fn db_path() -> String {
    std::env::var("HA_DB_PATH").unwrap_or_else(|_| DEFAULT_DB_PATH.to_owned())
}

async fn run_server() -> Result<(), Box<dyn Error>> {
    info!("ha-server {} (Ctrl+C pour arrêter)", ha_core::VERSION);

    let db_path = db_path();
    let history_days = history_days()?;
    let audit_days = days_from_env("HA_AUDIT_DAYS", DEFAULT_AUDIT_DAYS)?;
    let bind = bind_address()?;
    let mqtt_config = mqtt_config()?;
    // Un fichier d'automatisations invalide empêche le démarrage, avec un
    // message clair : mieux vaut corriger que de tourner à moitié.
    let file_automations = ha_automation::load_configs(&automations_path())?;
    let automations = ha_automation::check_all(&file_automations)?;
    let editable = automation_editor()?;
    let scripts = load_named(&scripts_path(), ha_automation::parse_scripts)?;
    let scenes = load_named(&scenes_path(), ha_automation::parse_scenes)?;
    let areas = ha_areas::load(&areas_path())
        .map_err(|error| format!("{} : {error}", areas_path().display()))?;
    info!("{} pièce(s) chargée(s)", areas.len());
    let location = location()?;
    let ntfy = ntfy_config()?;
    let alexa_settings = alexa_settings()?;
    let backup_settings = backup::settings()?;
    let weather_config = weather_config(location)?;
    let person_configs = ha_person::load(&persons_path())
        .map_err(|error| format!("{} : {error}", persons_path().display()))?;
    let input_config = ha_input::load(&inputs_path())
        .map_err(|error| format!("{} : {error}", inputs_path().display()))?;
    let thermostat_configs = ha_thermostat::load(&thermostats_path())
        .map_err(|error| format!("{} : {error}", thermostats_path().display()))?;
    if location.is_none() && ha_automation::uses_sun(&automations) {
        return Err(
            "des automatisations utilisent le soleil : indique la position de la maison \
                    avec HA_LATITUDE et HA_LONGITUDE"
                .into(),
        );
    }
    let store = open_store().await?;
    info!("base de données : {db_path}");

    // On oublie l'historique et le journal d'audit trop anciens.
    let purged = store.purge_history(days_ago(history_days)).await?;
    info!("historique conservé : {history_days} jours ({purged} lignes anciennes supprimées)");
    // Le journal de bord est gardé aussi longtemps que l'historique.
    store.purge_logbook(days_ago(history_days)).await?;
    let purged = store.purge_audit(days_ago(audit_days)).await?;
    info!("journal d'audit conservé : {audit_days} jours ({purged} lignes anciennes supprimées)");
    // Le journal a-t-il été modifié pendant que le serveur était arrêté ?
    match store.verify_audit().await? {
        ha_storage::AuditCheck::Intact { entries } => {
            info!("journal d'audit vérifié : {entries} lignes intactes");
        }
        ha_storage::AuditCheck::Broken { id, reason } => {
            warn!("journal d'audit MODIFIÉ à la ligne {id} : {reason}");
            warn!("quelqu'un a peut-être touché à la base : vérifie avec « ha-server audit »");
        }
    }
    cli::audit_local(&store, "serveur", "server_start", None).await;

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
    // Le journal de bord, avant tout ce qui peut agir sur la maison.
    let logbook = ha_logbook::Logbook::start(store.clone(), Arc::clone(&core));
    if let Err(error) = store
        .add_logbook(ha_storage::NewLogbookEntry {
            entity_id: None,
            name: "Serveur",
            message: "a démarré",
            cause: None,
        })
        .await
    {
        warn!("journal de bord : écriture impossible ({error})");
    }

    // API HTTP / WebSocket.
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!("API sur http://{bind}/api/");
    // L'éditeur d'automatisations : celles du fichier, plus celles créées
    // depuis le tableau de bord. Chaque modification recharge le moteur.
    let (reloader, reloads) = ha_automation::reload_channel();
    let editor = Arc::new(
        ha_automation::Editor::new(store.clone(), file_automations, location.is_some())
            .with_reloader(reloader),
    );
    let automations = editor.active().await?;
    if !editable {
        info!("éditeur d'automatisations en lecture seule (HA_AUTOMATION_EDITOR=off)");
    }
    let api_state = ApiState::new(
        Arc::clone(&core),
        store.clone(),
        auth,
        areas,
        Some(ha_api::AutomationEditor { editor, editable }),
    );
    let api = tokio::spawn(ha_api::serve(listener, Arc::clone(&api_state)));

    // On s'abonne AVANT de créer les entités, pour ne rater aucun événement.
    let logger = tokio::spawn(log_state_changes(core.bus.subscribe()));

    // Les entrées sont créées AVANT les automatisations (leur création n'est
    // pas un changement qui déclenche), et démarrées APRÈS (un minuteur qui a
    // sonné pendant l'arrêt doit atteindre les automatisations).
    let inputs = (!input_config.is_empty()).then(|| ha_input::Inputs::prepare(input_config, &core));

    // Vrais appareils (MQTT) ou, à défaut, appareils simulés.
    let mqtt = mqtt_config.map(|config| Mqtt::start(config, Arc::clone(&core)));
    let engine = Engine::start_with(
        ha_automation::Setup {
            automations,
            scripts,
            scenes,
            location,
            reloads: Some(reloads),
        },
        Arc::clone(&core),
    );
    let sun = location.map(|location| SunTracker::start(location, Arc::clone(&core)));
    let inputs = inputs.map(|inputs| inputs.start(Arc::clone(&core)));
    // Après la restauration des états : chaque thermostat reprend sa
    // consigne et son mode d'avant l'arrêt.
    let persons = (!person_configs.is_empty())
        .then(|| ha_person::Persons::start(person_configs, Arc::clone(&core)));
    let thermostats = (!thermostat_configs.is_empty())
        .then(|| ha_thermostat::Thermostats::start(thermostat_configs, Arc::clone(&core)));
    let backups = if let Some(settings) = backup_settings {
        Some(backup::Scheduler::start(
            settings,
            store.clone(),
            Arc::clone(&core),
        ))
    } else {
        info!("sauvegardes automatiques désactivées (indique HA_BACKUP_DIR pour les activer)");
        None
    };
    let weather = match weather_config {
        Some(config) => Some(ha_weather::Weather::start(config, Arc::clone(&core))?),
        None => None,
    };
    let notifier = match ntfy {
        Some(config) => {
            for warning in config.warnings() {
                warn!("notifications : {warning}");
            }
            Some(ha_notify::Notifier::start(config, Arc::clone(&core))?)
        }
        None => None,
    };
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
    let freebox = match ha_freebox::pairing::load(&store).await {
        Ok(pairing) => {
            pairing.map(|pairing| Freebox::start(pairing, store.clone(), Arc::clone(&core)))
        }
        Err(error) => {
            warn!("Freebox : association illisible ({error}). Refais « ha-server freebox pair ».");
            None
        }
    };
    // Alexa : un souci (port 80 déjà pris, droits insuffisants) ne doit pas
    // empêcher le reste de la maison de fonctionner.
    let alexa = match alexa_settings {
        Some(settings) => {
            if settings.devices.is_empty() {
                warn!(
                    "Alexa : aucun appareil dans {} : rien à piloter",
                    alexa_path().display()
                );
            }
            match ha_alexa::Alexa::start(settings, store.clone(), Arc::clone(&core)).await {
                Ok(alexa) => Some(alexa),
                Err(error) => {
                    warn!("Alexa désactivée : {error}");
                    warn!("voir docs/alexa.md (port 80, découverte sur le port 1900)");
                    None
                }
            }
        }
        None => None,
    };
    let mut simulation = if mqtt.is_none() && hue.is_none() && somfy.is_none() && freebox.is_none()
    {
        info!("aucun appareil configuré (MQTT, Hue, Somfy, Freebox) : appareils simulés");
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
    if let Some(alexa) = alexa {
        alexa.stop().await;
    }
    if let Some(backups) = backups {
        backups.stop().await;
    }
    if let Some(thermostats) = thermostats {
        thermostats.stop().await;
    }
    if let Some(persons) = persons {
        persons.stop().await;
    }
    if let Some(inputs) = inputs {
        inputs.stop().await;
    }
    engine.stop().await;
    logbook.stop().await;
    if let Some(notifier) = notifier {
        notifier.stop().await;
    }
    if let Some(sun) = sun {
        sun.stop().await;
    }
    if let Some(weather) = weather {
        weather.stop().await;
    }
    if let Some(freebox) = freebox {
        freebox.stop().await;
    }
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
    cli::audit_local(&store, "serveur", "server_stop", None).await;
    store.close().await;
    Ok(())
}

/// Lit `HA_HISTORY_DAYS`. Une valeur invalide arrête le programme avec un
/// message clair, plutôt que d'être ignorée en silence.
fn history_days() -> Result<u32, String> {
    days_from_env("HA_HISTORY_DAYS", DEFAULT_HISTORY_DAYS)
}

/// Lit un nombre de jours (entre 1 et 3650) dans la variable `name`.
fn days_from_env(name: &str, default: u32) -> Result<u32, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(value) => value
            .parse::<u32>()
            .ok()
            .filter(|days| (1..=3650).contains(days))
            .ok_or_else(|| format!("{name} invalide : {value:?} (entre 1 et 3650)")),
    }
}

/// Date d'il y a `days` jours.
fn days_ago(days: u32) -> SystemTime {
    SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(days) * 24 * 3600))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// `HA_AUTOMATION_EDITOR` : peut-on modifier les automatisations depuis le
/// tableau de bord ?
fn automation_editor() -> Result<bool, String> {
    match std::env::var("HA_AUTOMATION_EDITOR").as_deref() {
        Err(_) | Ok("on") => Ok(true),
        Ok("off") => Ok(false),
        Ok(other) => Err(format!(
            "HA_AUTOMATION_EDITOR : « {other} » invalide (on ou off)"
        )),
    }
}

/// `HA_METEO=on` : météo locale avec Open-Meteo. Il faut la position.
fn weather_config(location: Option<Location>) -> Result<Option<ha_weather::Config>, String> {
    match std::env::var("HA_METEO").as_deref() {
        Err(_) | Ok("off") => Ok(None),
        Ok("on") => {
            let location = location.ok_or(
                "HA_METEO=on : indique la position de la maison avec HA_LATITUDE et HA_LONGITUDE",
            )?;
            let url = std::env::var("HA_METEO_URL").ok();
            ha_weather::Config::new(location, url.as_deref())
                .map(Some)
                .map_err(|error| error.to_string())
        }
        Ok(other) => Err(format!("HA_METEO : « {other} » invalide (on ou off)")),
    }
}

pub(crate) fn persons_path() -> std::path::PathBuf {
    std::env::var("HA_PERSONS_PATH")
        .unwrap_or_else(|_| "persons.yaml".to_owned())
        .into()
}

pub(crate) fn inputs_path() -> std::path::PathBuf {
    std::env::var("HA_INPUTS_PATH")
        .unwrap_or_else(|_| "inputs.yaml".to_owned())
        .into()
}

pub(crate) fn thermostats_path() -> std::path::PathBuf {
    std::env::var("HA_THERMOSTATS_PATH")
        .unwrap_or_else(|_| "thermostats.yaml".to_owned())
        .into()
}

pub(crate) fn alexa_path() -> std::path::PathBuf {
    std::env::var("HA_ALEXA_PATH")
        .unwrap_or_else(|_| "alexa.yaml".to_owned())
        .into()
}

/// Réglages d'Alexa (`HA_ALEXA_IP`, `HA_ALEXA_PORT`, `HA_ALEXA_ALLOWED`).
/// `None` si `HA_ALEXA_IP` n'est pas indiquée : Alexa est désactivée.
fn alexa_settings() -> Result<Option<ha_alexa::Settings>, String> {
    let Ok(ip) = std::env::var("HA_ALEXA_IP") else {
        return Ok(None);
    };
    let ip: std::net::Ipv4Addr = ip.trim().parse().map_err(|_| {
        format!("HA_ALEXA_IP invalide : {ip:?} (l'adresse IPv4 de cette machine, par exemple 192.168.1.20)")
    })?;
    if ip.is_unspecified() || ip.is_loopback() {
        return Err(format!(
            "HA_ALEXA_IP : {ip} ne convient pas, il faut l'adresse de cette machine sur le réseau local"
        ));
    }
    let port = match std::env::var("HA_ALEXA_PORT") {
        Err(_) => ha_alexa::DEFAULT_PORT,
        Ok(value) => value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| format!("HA_ALEXA_PORT invalide : {value:?}"))?,
    };
    let allowed = match std::env::var("HA_ALEXA_ALLOWED") {
        Err(_) => ha_alexa::Allowed::LocalNetwork,
        Ok(list) => ha_alexa::Allowed::Only(
            list.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| {
                    item.parse()
                        .map_err(|_| format!("HA_ALEXA_ALLOWED : adresse invalide {item:?}"))
                })
                .collect::<Result<_, _>>()?,
        ),
    };
    let devices = ha_alexa::load(&alexa_path())
        .map_err(|error| format!("{} : {error}", alexa_path().display()))?;
    Ok(Some(ha_alexa::Settings {
        ip,
        port,
        discovery_port: ha_alexa::ssdp::PORT,
        allowed,
        devices,
    }))
}

/// Lit la position de la maison (`HA_LATITUDE`, `HA_LONGITUDE`), pour le
/// soleil. `None` si elle n'est pas indiquée.
pub(crate) fn location() -> Result<Option<Location>, String> {
    let read = |name: &str| -> Result<Option<f64>, String> {
        match std::env::var(name) {
            Err(_) => Ok(None),
            Ok(value) => value
                .trim()
                .parse::<f64>()
                .map(Some)
                .map_err(|_| format!("{name} invalide : {value:?} (exemple : 48.85)")),
        }
    };
    match (read("HA_LATITUDE")?, read("HA_LONGITUDE")?) {
        (None, None) => Ok(None),
        (Some(latitude), Some(longitude)) => {
            let location = Location::new(latitude, longitude).map_err(|error| error.to_string())?;
            // La position exacte de la maison est une donnée personnelle :
            // on ne l'écrit pas dans les logs.
            info!("soleil : position de la maison configurée");
            Ok(Some(location))
        }
        _ => Err("il faut HA_LATITUDE et HA_LONGITUDE ensemble".into()),
    }
}

/// Lit la configuration des notifications (`HA_NTFY_URL`, `HA_NTFY_TOPIC`,
/// `HA_NTFY_TOKEN_FILE` ou `HA_NTFY_TOKEN`). `None` sans `HA_NTFY_URL`.
fn ntfy_config() -> Result<Option<ha_notify::Config>, String> {
    let Ok(url) = std::env::var("HA_NTFY_URL") else {
        return Ok(None);
    };
    let topic = std::env::var("HA_NTFY_TOPIC").map_err(|_| "HA_NTFY_TOPIC manquant")?;
    // Comme pour MQTT, un fichier est préférable à une variable
    // d'environnement pour un secret.
    let token = match std::env::var("HA_NTFY_TOKEN_FILE") {
        Ok(path) => {
            let content =
                Zeroizing::new(std::fs::read_to_string(&path).map_err(|error| {
                    format!("lecture de HA_NTFY_TOKEN_FILE ({path}) : {error}")
                })?);
            Some(Zeroizing::new(content.trim().to_owned()))
        }
        Err(_) => std::env::var("HA_NTFY_TOKEN").ok().map(Zeroizing::new),
    };
    ha_notify::Config::new(&url, &topic, token)
        .map(Some)
        .map_err(|error| error.to_string())
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
pub(crate) fn parse_bind() -> Result<SocketAddr, String> {
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
                Event::Fired { event_type, .. } => info!("événement {event_type}"),
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
