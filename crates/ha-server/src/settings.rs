//! Les réglages du serveur, lus dans les variables d'environnement
//! (`HA_MQTT_HOST`, `HA_BIND`, `HA_METEO`…).
//!
//! Une valeur invalide arrête le programme avec un message clair, plutôt
//! que d'être ignorée en silence. Quand c'est possible, la vérification est
//! une fonction « pure » (elle reçoit la valeur au lieu de lire la
//! variable) : on peut l'essayer dans les tests sans toucher à
//! l'environnement, qui est commun à tous les tests lancés en même temps.

use std::net::SocketAddr;

use ha_mqtt::MqttConfig;
use ha_somfy::CloudAccount;
use ha_sun::Location;
use tracing::{info, warn};
use zeroize::Zeroizing;

const DEFAULT_BIND: &str = "127.0.0.1:8123";
const DEFAULT_HISTORY_DAYS: u32 = 10;

/// Lit `HA_HISTORY_DAYS`. Une valeur invalide arrête le programme avec un
/// message clair, plutôt que d'être ignorée en silence.
pub(crate) fn history_days() -> Result<u32, String> {
    days_from_env("HA_HISTORY_DAYS", DEFAULT_HISTORY_DAYS)
}

/// Lit un nombre de jours (entre 1 et 3650) dans la variable `name`.
pub(crate) fn days_from_env(name: &str, default: u32) -> Result<u32, String> {
    parse_days(name, std::env::var(name).ok().as_deref(), default)
}

/// Vérifie un nombre de jours ; `value` vaut `None` si la variable est
/// absente.
fn parse_days(name: &str, value: Option<&str>, default: u32) -> Result<u32, String> {
    match value {
        None => Ok(default),
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|days| (1..=3650).contains(days))
            .ok_or_else(|| format!("{name} invalide : {value:?} (entre 1 et 3650)")),
    }
}

/// `HA_AUTOMATION_EDITOR` : peut-on modifier les automatisations depuis le
/// tableau de bord ?
pub(crate) fn automation_editor() -> Result<bool, String> {
    parse_automation_editor(std::env::var("HA_AUTOMATION_EDITOR").ok().as_deref())
}

/// `on` (ou rien) : oui ; `off` : non.
fn parse_automation_editor(value: Option<&str>) -> Result<bool, String> {
    match value {
        None | Some("on") => Ok(true),
        Some("off") => Ok(false),
        Some(other) => Err(format!(
            "HA_AUTOMATION_EDITOR : « {other} » invalide (on ou off)"
        )),
    }
}

/// `HA_METEO=on` : météo locale avec Open-Meteo. Il faut la position.
pub(crate) fn weather_config(
    location: Option<Location>,
) -> Result<Option<ha_weather::Config>, String> {
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

/// La vigilance Météo-France : `HA_VIGILANCE_DEPARTEMENT` et la clé
/// (`HA_VIGILANCE_KEY_FILE`, conseillé, ou `HA_VIGILANCE_KEY`).
pub(crate) fn vigilance_config() -> Result<Option<ha_weather::vigilance::VigilanceConfig>, String> {
    let Ok(department) = std::env::var("HA_VIGILANCE_DEPARTEMENT") else {
        return Ok(None);
    };
    // Comme pour MQTT et ntfy, un fichier est préférable à une variable
    // d'environnement pour un secret.
    let key = match std::env::var("HA_VIGILANCE_KEY_FILE") {
        Ok(path) => Zeroizing::new(
            std::fs::read_to_string(&path)
                .map_err(|error| format!("lecture de HA_VIGILANCE_KEY_FILE ({path}) : {error}"))?,
        ),
        Err(_) => std::env::var("HA_VIGILANCE_KEY")
            .map(Zeroizing::new)
            .map_err(|_| {
                "HA_VIGILANCE_DEPARTEMENT : il faut aussi la clé de l'API Météo-France \
             (HA_VIGILANCE_KEY_FILE), voir docs/vigilance.md"
            })?,
    };
    let url = std::env::var("HA_VIGILANCE_URL").ok();
    ha_weather::vigilance::VigilanceConfig::new(&department, &key, url.as_deref())
        .map(Some)
        .map_err(|error| error.to_string())
}

pub(crate) fn persons_path() -> std::path::PathBuf {
    std::env::var("HA_PERSONS_PATH")
        .unwrap_or_else(|_| "persons.yaml".to_owned())
        .into()
}

pub(crate) fn energy_path() -> std::path::PathBuf {
    std::env::var("HA_ENERGY_PATH")
        .unwrap_or_else(|_| "energy.yaml".to_owned())
        .into()
}

/// Les adresses de Firebase pour Iglou. `HA_IGLOU_API` ne sert qu'aux essais,
/// avec un faux Firebase sur ce PC (`http://127.0.0.1:…`) : toute autre
/// adresse est ignorée.
pub(crate) fn iglou_endpoints() -> ha_iglou::Endpoints {
    std::env::var("HA_IGLOU_API")
        .ok()
        .filter(|url| url.starts_with("http://127.0.0.1:"))
        .map_or_else(ha_iglou::Endpoints::google, |url| {
            ha_iglou::Endpoints::local(&url)
        })
}

/// L'adresse de l'API TimeTree. `HA_TIMETREE_API` ne sert qu'aux essais, avec
/// un faux TimeTree sur ce PC (`http://127.0.0.1:…`) : toute autre adresse
/// est ignorée.
pub(crate) fn timetree_api() -> String {
    std::env::var("HA_TIMETREE_API")
        .ok()
        .filter(|url| url.starts_with("http://127.0.0.1:"))
        .unwrap_or_else(|| ha_ics::timetree::API.to_owned())
}

/// Où joindre SmartThings. `HA_SMARTTHINGS_API` ne sert qu'aux essais, avec
/// un faux SmartThings sur ce PC (`http://127.0.0.1:…`) : toute autre
/// adresse est ignorée.
pub(crate) fn smartthings_endpoint() -> ha_smartthings::api::Endpoint {
    std::env::var("HA_SMARTTHINGS_API")
        .ok()
        .and_then(|url| ha_smartthings::api::Endpoint::local(&url))
        .unwrap_or_else(ha_smartthings::api::Endpoint::smartthings)
}

/// Où lire les e-mails des colis. `HA_COLIS_IMAP_TEST` (et
/// `HA_COLIS_IMAGES_TEST`) ne servent qu'aux essais, avec un faux serveur
/// sur ce PC (`127.0.0.1:…`) : toute autre adresse est ignorée.
pub(crate) fn colis_mailbox() -> ha_colis::Mailbox {
    std::env::var("HA_COLIS_IMAP_TEST")
        .ok()
        .and_then(|imap| {
            let images = std::env::var("HA_COLIS_IMAGES_TEST").ok();
            ha_colis::Mailbox::local(&imap, images.as_deref())
        })
        .unwrap_or_else(ha_colis::Mailbox::real)
}

/// `HA_SONOS` : les enceintes (vide : les chercher).
pub(crate) fn sonos_hosts() -> Result<Vec<String>, String> {
    ha_sonos::parse_hosts(&std::env::var("HA_SONOS").unwrap_or_default())
}

/// `HA_PIPER` et `HA_PIPER_VOIX` : la voix des annonces. Une seule voix
/// pour tout le serveur (enceintes et tablette) : un seul Piper gardé en
/// marche.
pub(crate) fn tts_config() -> Result<Option<ha_tts::Tts>, String> {
    static TTS: std::sync::OnceLock<Result<Option<ha_tts::Tts>, String>> =
        std::sync::OnceLock::new();
    TTS.get_or_init(|| {
        let Ok(piper) = std::env::var("HA_PIPER") else {
            return Ok(None);
        };
        let voice = std::env::var("HA_PIPER_VOIX").ok();
        ha_tts::Tts::from_settings(&piper, voice.as_deref())
            .map(Some)
            .map_err(|error| error.to_string())
    })
    .clone()
}

/// L'écoute de la tablette, si `HA_ECOUTE` est là : l'adresse du service
/// d'écoute (`ws://127.0.0.1:10350`) ou le dossier du programme.
pub(crate) fn stt_config() -> Result<Option<ha_stt::Stt>, String> {
    let Some(setting) = std::env::var("HA_ECOUTE")
        .ok()
        .filter(|setting| !setting.trim().is_empty())
    else {
        return Ok(None);
    };
    ha_stt::Stt::from_settings(&setting)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Les réglages Sonos, si `HA_SONOS` est là.
pub(crate) fn sonos_config() -> Result<Option<ha_sonos::Config>, String> {
    if std::env::var_os("HA_SONOS").is_none() {
        return Ok(None);
    }
    let mut config = ha_sonos::Config::new(sonos_hosts()?);
    if let Ok(audio) = std::env::var("HA_SONOS_AUDIO") {
        let (address, port) = ha_sonos::parse_audio(&audio)?;
        config.audio_address = Some(address.into());
        config.audio_port = port;
    }
    // Une voix mal installée n'empêche pas le reste : les annonces se
    // limitent alors au carillon, et le journal dit pourquoi.
    config.tts = match tts_config() {
        Ok(tts) => tts,
        Err(error) => {
            warn!("voix des annonces : {error}");
            None
        }
    };
    Ok(Some(config))
}

/// Le compteur Linky (`HA_TELEINFO`), s'il est branché.
pub(crate) fn teleinfo_config() -> Result<Option<ha_teleinfo::Config>, String> {
    let Ok(source) = std::env::var("HA_TELEINFO") else {
        return Ok(None);
    };
    let mode = std::env::var("HA_TELEINFO_MODE").ok();
    ha_teleinfo::Config::parse(&source, mode.as_deref())
        .map(Some)
        .map_err(|error| format!("HA_TELEINFO : {error}"))
}

pub(crate) fn derived_path() -> std::path::PathBuf {
    std::env::var("HA_DERIVED_PATH")
        .unwrap_or_else(|_| "derived.yaml".to_owned())
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

/// Réglages d'Alexa (`HA_ALEXA_IP`, `HA_ALEXA_PORT`, `HA_ALEXA_ALLOWED`,
/// `HA_ALEXA_LISTEN`). `None` si `HA_ALEXA_IP` n'est pas indiquée : Alexa est
/// désactivée.
pub(crate) fn alexa_settings() -> Result<Option<ha_alexa::Settings>, String> {
    let Ok(ip) = std::env::var("HA_ALEXA_IP") else {
        return Ok(None);
    };
    let (ip, port, allowed) = alexa_network(&ip)?;
    let listen = match std::env::var("HA_ALEXA_LISTEN") {
        Err(_) => None,
        Ok(value) => Some(value.trim().parse().map_err(|_| {
            format!("HA_ALEXA_LISTEN invalide : {value:?} (par exemple 0.0.0.0:8080)")
        })?),
    };
    let devices = ha_alexa::load(&alexa_path())
        .map_err(|error| format!("{} : {error}", alexa_path().display()))?;
    Ok(Some(ha_alexa::Settings {
        ip,
        port,
        listen,
        discovery_port: ha_alexa::ssdp::PORT,
        allowed,
        devices,
    }))
}

/// Réglages du relais (`ha-server alexa relay [pont]`) : les mêmes variables
/// que le pont, et l'adresse du pont dans le conteneur.
pub(crate) fn relay_settings(args: &[&str]) -> Result<ha_alexa::relay::RelaySettings, String> {
    let target = match args {
        [] => "127.0.0.1:8127",
        [target] => target,
        _ => return Err("usage : ha-server alexa relay [adresse du pont, 127.0.0.1:8127]".into()),
    };
    let target = target.parse().map_err(|_| {
        format!("adresse du pont invalide : {target:?} (par exemple 127.0.0.1:8127)")
    })?;
    let ip = std::env::var("HA_ALEXA_IP").map_err(|_| {
        "indique HA_ALEXA_IP, l'adresse de cette machine sur le réseau local : voir docs/alexa.md"
            .to_owned()
    })?;
    let (ip, port, allowed) = alexa_network(&ip)?;
    Ok(ha_alexa::relay::RelaySettings {
        ip,
        port,
        discovery_port: ha_alexa::ssdp::PORT,
        target,
        allowed,
    })
}

/// Adresse, port et Echo autorisés, communs au pont et au relais.
fn alexa_network(ip: &str) -> Result<(std::net::Ipv4Addr, u16, ha_alexa::Allowed), String> {
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
    Ok((ip, port, allowed))
}

/// Lit la position de la maison (`HA_LATITUDE`, `HA_LONGITUDE`), pour le
/// soleil. `None` si elle n'est pas indiquée.
pub(crate) fn location() -> Result<Option<Location>, String> {
    let location = parse_location(
        std::env::var("HA_LATITUDE").ok().as_deref(),
        std::env::var("HA_LONGITUDE").ok().as_deref(),
    )?;
    if location.is_some() {
        // La position exacte de la maison est une donnée personnelle :
        // on ne l'écrit pas dans les logs.
        info!("soleil : position de la maison configurée");
    }
    Ok(location)
}

/// Vérifie la latitude et la longitude : les deux, ou aucune.
fn parse_location(
    latitude: Option<&str>,
    longitude: Option<&str>,
) -> Result<Option<Location>, String> {
    let read = |name: &str, value: Option<&str>| -> Result<Option<f64>, String> {
        match value {
            None => Ok(None),
            Some(value) => value
                .trim()
                .parse::<f64>()
                .map(Some)
                .map_err(|_| format!("{name} invalide : {value:?} (exemple : 48.85)")),
        }
    };
    match (
        read("HA_LATITUDE", latitude)?,
        read("HA_LONGITUDE", longitude)?,
    ) {
        (None, None) => Ok(None),
        (Some(latitude), Some(longitude)) => Location::new(latitude, longitude)
            .map(Some)
            .map_err(|error| error.to_string()),
        _ => Err("il faut HA_LATITUDE et HA_LONGITUDE ensemble".into()),
    }
}

/// Lit la configuration des notifications (`HA_NTFY_URL`, `HA_NTFY_TOPIC`,
/// `HA_NTFY_TOKEN_FILE` ou `HA_NTFY_TOKEN`). `None` sans `HA_NTFY_URL`.
pub(crate) fn ntfy_config() -> Result<Option<ha_notify::Config>, String> {
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

/// Le compte Somfy pour le chauffage Atlantic : `HA_SOMFY_EMAIL` et le mot
/// de passe dans un fichier (`HA_SOMFY_PASSWORD_FILE`), hors du dépôt.
pub(crate) fn somfy_account() -> Result<Option<CloudAccount>, String> {
    let Ok(email) = std::env::var("HA_SOMFY_EMAIL") else {
        return Ok(None);
    };
    let email = email.trim().to_owned();
    if email.is_empty() || email.len() > 200 || !email.contains('@') {
        return Err("HA_SOMFY_EMAIL invalide (exemple : moi@exemple.fr)".into());
    }
    // Comme pour MQTT : un fichier est préférable à une variable d'environnement.
    let password = match std::env::var("HA_SOMFY_PASSWORD_FILE") {
        Ok(path) => {
            let content = Zeroizing::new(std::fs::read_to_string(&path).map_err(|error| {
                format!("lecture de HA_SOMFY_PASSWORD_FILE ({path}) : {error}")
            })?);
            Zeroizing::new(content.trim_end_matches(['\r', '\n']).to_owned())
        }
        Err(_) => std::env::var("HA_SOMFY_PASSWORD")
            .map(Zeroizing::new)
            .map_err(|_| {
                "HA_SOMFY_EMAIL : il faut aussi le mot de passe Somfy \
                 (HA_SOMFY_PASSWORD_FILE), voir docs/somfy.md"
            })?,
    };
    if password.is_empty() {
        return Err("mot de passe Somfy vide (HA_SOMFY_PASSWORD_FILE)".into());
    }
    Ok(Some(CloudAccount::new(email, password)))
}

/// La clé du relais des notifications du téléphone (étape 103), lue dans
/// le fichier `HA_RELAIS_KEY_FILE`. Absente : le relais reste fermé.
pub(crate) fn relay_key() -> Result<Option<Zeroizing<String>>, String> {
    let Ok(path) = std::env::var("HA_RELAIS_KEY_FILE") else {
        return Ok(None);
    };
    let content = Zeroizing::new(
        std::fs::read_to_string(&path)
            .map_err(|error| format!("lecture de HA_RELAIS_KEY_FILE ({path}) : {error}"))?,
    );
    Ok(Some(Zeroizing::new(content.trim().to_owned())))
}

/// Lit la configuration MQTT. `None` si `HA_MQTT_HOST` n'est pas défini.
pub(crate) fn mqtt_config() -> Result<Option<MqttConfig>, String> {
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

/// Lit `HA_BIND`, sans rien afficher.
pub(crate) fn parse_bind() -> Result<SocketAddr, String> {
    parse_bind_value(std::env::var("HA_BIND").ok().as_deref())
}

/// Vérifie l'adresse d'écoute ; `None` : celle par défaut.
fn parse_bind_value(value: Option<&str>) -> Result<SocketAddr, String> {
    let value = value.unwrap_or(DEFAULT_BIND);
    value
        .parse()
        .map_err(|_| format!("HA_BIND invalide : {value:?} (exemple : 127.0.0.1:8123)"))
}

/// Lit `HA_BIND`. Par défaut, l'API n'écoute que sur la machine elle-même.
pub(crate) fn bind_address() -> Result<SocketAddr, String> {
    let addr = parse_bind()?;
    if !addr.ip().is_loopback() {
        warn!("l'API écoute sur {addr} : elle est joignable depuis le réseau.");
        warn!("ne l'expose JAMAIS directement sur Internet : passe par un VPN.");
    }
    Ok(addr)
}

/// `HA_JOURS_FERIES` et `HA_JOURS_OUVRES` (étape 56).
pub(crate) fn workday_settings() -> Result<(ha_workday::Regime, ha_workday::Week), String> {
    let regime = ha_workday::Regime::from_settings(
        std::env::var("HA_JOURS_FERIES").ok().as_deref(),
        std::env::var("HA_VIGILANCE_DEPARTEMENT").ok().as_deref(),
    )?;
    let week = match std::env::var("HA_JOURS_OUVRES") {
        Ok(days) => ha_workday::Week::parse(&days)?,
        Err(_) => ha_workday::Week::default(),
    };
    Ok((regime, week))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn days_are_between_1_and_3650() {
        assert_eq!(parse_days("HA_HISTORY_DAYS", None, 10), Ok(10));
        assert_eq!(parse_days("HA_HISTORY_DAYS", Some("30"), 10), Ok(30));
        assert_eq!(parse_days("HA_HISTORY_DAYS", Some("3650"), 10), Ok(3650));
        for wrong in ["0", "3651", "-1", "dix", ""] {
            assert!(parse_days("HA_HISTORY_DAYS", Some(wrong), 10).is_err());
        }
        assert_eq!(
            parse_days("HA_AUDIT_DAYS", Some("0"), 365),
            Err("HA_AUDIT_DAYS invalide : \"0\" (entre 1 et 3650)".to_owned())
        );
    }

    #[test]
    fn automation_editor_is_on_by_default() {
        assert_eq!(parse_automation_editor(None), Ok(true));
        assert_eq!(parse_automation_editor(Some("on")), Ok(true));
        assert_eq!(parse_automation_editor(Some("off")), Ok(false));
        assert!(parse_automation_editor(Some("oui")).is_err());
    }

    #[test]
    fn location_needs_both_values() {
        assert_eq!(parse_location(None, None), Ok(None));
        let location = parse_location(Some(" 48.85 "), Some("2.35")).unwrap();
        assert_eq!(location, Some(Location::new(48.85, 2.35).unwrap()));
        assert_eq!(
            parse_location(Some("48.85"), None),
            Err("il faut HA_LATITUDE et HA_LONGITUDE ensemble".to_owned())
        );
        assert!(parse_location(Some("nord"), Some("2.35")).is_err());
        // Hors des bornes : refusé par `Location::new`.
        assert!(parse_location(Some("91"), Some("2.35")).is_err());
    }

    #[test]
    fn bind_defaults_to_this_machine() {
        let addr = parse_bind_value(None).unwrap();
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), 8123);
        assert_eq!(parse_bind_value(Some("0.0.0.0:9000")).unwrap().port(), 9000);
        assert!(parse_bind_value(Some("8123")).is_err());
    }
}
