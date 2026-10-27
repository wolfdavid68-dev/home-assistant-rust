//! La **vigilance météo** de Météo-France pour le département de la maison :
//! vent violent, orages, canicule, neige-verglas… en vert, jaune, orange
//! ou rouge, aujourd'hui et demain.
//!
//! Les données viennent de l'API officielle « Données publiques Vigilance »
//! (<https://portail-api.meteofrance.fr>) : gratuite, mais il faut un compte
//! et une **clé**, envoyée dans l'en-tête `apikey`.
//!
//! On obtient deux capteurs, utilisables dans les automatisations :
//! - `sensor.vigilance_meteo` : le niveau du jour (`Vert`, `Jaune`,
//!   `Orange`, `Rouge`) ; en attributs, le niveau de chaque phénomène, ses
//!   plages horaires et le niveau en ce moment ;
//! - `sensor.vigilance_meteo_demain` : le niveau de demain (publié à
//!   partir de 6 h).
//!
//! Sécurité : la clé n'apparaît ni dans les logs, ni dans les messages
//! d'erreur, ni dans `Debug`. Si Météo-France la refuse (expirée, révoquée),
//! une alerte de réparation le dit sur le tableau de bord.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::{Attributes, Background, CancellationToken, Core};
use ha_notices::{Issue, Severity};
use jiff::Timestamp;
use reqwest::header::HeaderValue;
use serde_json::{Value, json};
use tracing::{info, warn};
use zeroize::Zeroizing;

use crate::carte::{self, Hazard, Map, Period};
use crate::{WeatherError, check_url, object, set};

/// La carte de vigilance en cours, sur l'API de Météo-France.
pub const DEFAULT_URL: &str =
    "https://public-api.meteofrance.fr/public/DPVigilance/v1/cartevigilance/encours";
/// La carte est publiée au moins à 6 h et à 16 h, et à chaque changement.
pub const DEFAULT_INTERVAL: Duration = Duration::from_mins(10);
const RETRY_DELAY: Duration = Duration::from_mins(5);
/// Une vigilance trop ancienne ne doit pas passer pour l'actuelle.
const STALE_AFTER: Duration = Duration::from_hours(3);
const MAX_KEY_LEN: usize = 4096;

pub const TODAY_ENTITY: &str = "sensor.vigilance_meteo";
pub const TOMORROW_ENTITY: &str = "sensor.vigilance_meteo_demain";
const ATTRIBUTION: &str = "Vigilance : Météo-France";
const REPAIR_PREFIX: &str = "repair_vigilance_";

/// Un département de métropole : `01` à `95`, `2A` ou `2B`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Department(String);

impl Department {
    /// # Errors
    /// Ce n'est pas un département de métropole.
    pub fn parse(text: &str) -> Result<Self, WeatherError> {
        let code = text.trim().to_ascii_uppercase();
        let valid = match code.as_str() {
            "2A" | "2B" => true,
            _ => {
                code.len() == 2
                    && code
                        .parse::<u8>()
                        .is_ok_and(|n| (1..=95).contains(&n) && n != 20)
            }
        };
        if valid {
            return Ok(Self(code));
        }
        let overseas = code.len() == 3 && code.starts_with("97");
        Err(WeatherError::Vigilance(if overseas {
            "les départements d'outre-mer ont leur propre vigilance, pas encore prise en charge"
                .into()
        } else {
            format!("département {text:?} invalide (deux chiffres comme 67, ou 2A, 2B)")
        }))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Réglages de la vigilance.
#[derive(Clone)]
pub struct VigilanceConfig {
    url: url::Url,
    department: Department,
    key: Zeroizing<String>,
    pub interval: Duration,
    pub stale_after: Duration,
}

/// Concept Rust : on écrit `Debug` à la main pour que la clé n'apparaisse
/// jamais, même dans un `{:?}` oublié dans un log.
impl fmt::Debug for VigilanceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VigilanceConfig")
            .field("url", &self.url.as_str())
            .field("department", &self.department)
            .field("key", &"<secret>")
            .field("interval", &self.interval)
            .finish_non_exhaustive()
    }
}

impl VigilanceConfig {
    /// `url` : `None` pour l'API de Météo-France (sinon, pour les tests, un
    /// serveur du réseau local).
    ///
    /// # Errors
    /// Département, clé ou adresse invalide.
    pub fn new(department: &str, key: &str, url: Option<&str>) -> Result<Self, WeatherError> {
        let department = Department::parse(department)?;
        let trimmed = Zeroizing::new(key.trim().to_owned());
        // La clé part dans un en-tête HTTP : des caractères visibles seulement.
        if trimmed.is_empty()
            || trimmed.len() > MAX_KEY_LEN
            || !trimmed.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(WeatherError::Vigilance(
                "clé de l'API Météo-France vide ou invalide".into(),
            ));
        }
        Ok(Self {
            url: check_url(url.unwrap_or(DEFAULT_URL))?,
            department,
            key: trimmed,
            interval: DEFAULT_INTERVAL,
            stale_after: STALE_AFTER,
        })
    }

    #[must_use]
    pub fn department(&self) -> &Department {
        &self.department
    }
}

/// Le service de vigilance, en marche.
#[derive(Debug)]
pub struct Vigilance(Background);

impl Vigilance {
    /// # Errors
    /// Le client HTTPS n'a pas pu être préparé.
    pub fn start(config: VigilanceConfig, core: Arc<Core>) -> Result<Self, ha_tls::TlsError> {
        let http = ha_tls::web_client(config.url.scheme() == "https")?;
        Ok(Self(Background::spawn(
            "la vigilance météo",
            move |token| run(config, http, core, token),
        )))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Pourquoi une requête a échoué.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// Météo-France refuse la clé (401, 403).
    Refused,
    Other(String),
}

async fn run(
    config: VigilanceConfig,
    http: reqwest::Client,
    core: Arc<Core>,
    token: CancellationToken,
) {
    info!(
        "vigilance météo : département {}, toutes les {} min",
        config.department.as_str(),
        config.interval.as_secs() / 60
    );
    let mut last: Option<(Map, Instant)> = None;
    let mut failing = false;
    loop {
        let delay = match fetch(&http, &config).await {
            Ok(map) => {
                if failing {
                    info!("vigilance météo : de nouveau disponible");
                }
                failing = false;
                ha_notices::reconcile(&core.states, REPAIR_PREFIX, &[]);
                last = Some((map, Instant::now()));
                config.interval
            }
            Err(failure) => {
                if !failing {
                    match &failure {
                        Failure::Refused => warn!("vigilance météo : clé refusée par Météo-France"),
                        Failure::Other(error) => {
                            warn!("vigilance météo : {error} (nouvel essai dans quelques minutes)");
                        }
                    }
                }
                failing = true;
                let issues = if failure == Failure::Refused {
                    vec![refused_key()]
                } else {
                    Vec::new()
                };
                ha_notices::reconcile(&core.states, REPAIR_PREFIX, &issues);
                RETRY_DELAY.min(config.interval)
            }
        };
        // Même sans nouvelle carte, on recalcule : à minuit, « demain »
        // devient aujourd'hui.
        let fresh = last
            .as_ref()
            .filter(|(_, at)| at.elapsed() < config.stale_after)
            .map(|(map, _)| map);
        match fresh {
            Some(map) => publish(&core, &config.department, map, Timestamp::now()),
            None if failing => mark_unavailable(&core),
            None => {}
        }
        tokio::select! {
            () = token.cancelled() => break,
            () = tokio::time::sleep(delay) => {}
        }
    }
}

fn refused_key() -> Issue {
    Issue {
        key: "cle".into(),
        title: "Vigilance météo : clé refusée".into(),
        message: "Météo-France refuse la clé de l'API (HA_VIGILANCE_KEY_FILE) : elle a peut-être \
                  expiré. Crée une nouvelle clé sur portail-api.meteofrance.fr (voir \
                  docs/vigilance.md), puis redémarre le serveur."
            .into(),
        severity: Severity::Error,
    }
}

/// Une seule requête, pour vérifier la clé (`ha-server vigilance test`).
///
/// # Errors
/// Clé refusée, réseau, ou carte illisible.
pub async fn fetch_once(config: &VigilanceConfig) -> Result<Map, String> {
    let http = ha_tls::web_client(config.url.scheme() == "https").map_err(|e| e.to_string())?;
    fetch(&http, config).await.map_err(|failure| match failure {
        Failure::Refused => "clé refusée par Météo-France : vérifie-la, ou crée-en une nouvelle \
                             sur portail-api.meteofrance.fr"
            .into(),
        Failure::Other(error) => error,
    })
}

/// Une requête à Météo-France.
async fn fetch(http: &reqwest::Client, config: &VigilanceConfig) -> Result<Map, Failure> {
    let describe = |error: reqwest::Error| Failure::Other(error.without_url().to_string());
    let mut key =
        HeaderValue::from_str(&config.key).map_err(|_| Failure::Other("clé invalide".into()))?;
    // La valeur ne sera jamais affichée par `reqwest` (logs, erreurs).
    key.set_sensitive(true);
    let response = http
        .get(config.url.clone())
        .header("apikey", key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(describe)?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(Failure::Refused);
    }
    if !status.is_success() {
        return Err(Failure::Other(format!("erreur HTTP {status}")));
    }
    let body = ha_tls::read_limited(response, carte::MAX_BODY)
        .await
        .map_err(|error| match error {
            ha_tls::ReadError::Http(error) => describe(error),
            ha_tls::ReadError::TooLarge => Failure::Other(error.to_string()),
        })?;
    carte::parse(&body, config.department.as_str()).map_err(Failure::Other)
}

/// Met à jour les deux capteurs.
fn publish(core: &Core, department: &Department, map: &Map, now: Timestamp) {
    let Some(today) = map.at(now) else {
        // La carte ne couvre plus l'heure actuelle : elle est périmée.
        mark_unavailable(core);
        return;
    };
    let mut attributes = period_attributes("Vigilance météo", department, map, today);
    attributes.insert("niveau_actuel".into(), today.level_at(now).name().into());
    set(core, TODAY_ENTITY, today.level.name(), attributes);
    match map.after(today) {
        Some(tomorrow) => set(
            core,
            TOMORROW_ENTITY,
            tomorrow.level.name(),
            period_attributes("Vigilance météo demain", department, map, tomorrow),
        ),
        None => set(
            core,
            TOMORROW_ENTITY,
            "unknown",
            base_attributes("Vigilance météo demain", department),
        ),
    }
}

fn base_attributes(name: &str, department: &Department) -> Attributes {
    object(json!({
        "friendly_name": name,
        "icon": "mdi:alert",
        "device_class": "enum",
        "options": ["Vert", "Jaune", "Orange", "Rouge"],
        "departement": department.as_str(),
        "attribution": ATTRIBUTION,
    }))
}

fn period_attributes(
    name: &str,
    department: &Department,
    map: &Map,
    period: &Period,
) -> Attributes {
    let mut attributes = base_attributes(name, department);
    attributes.insert("debut".into(), period.begin.to_string().into());
    attributes.insert("fin".into(), period.end.to_string().into());
    attributes.insert("mise_a_jour".into(), map.updated.to_string().into());
    // Comme dans Home Assistant : un attribut par phénomène suivi
    // (`Orages: Jaune`), pratique dans les conditions d'automatisations.
    for hazard in &period.hazards {
        attributes.insert(hazard.phenomenon.name().into(), hazard.level.name().into());
    }
    let active: Vec<Value> = period
        .hazards
        .iter()
        .filter(|hazard| hazard.level > carte::Level::Green)
        .map(hazard_json)
        .collect();
    attributes.insert("phenomenes".into(), active.into());
    attributes
}

/// Un phénomène au-dessus du vert, avec ses plages horaires (au-dessus du vert).
fn hazard_json(hazard: &Hazard) -> Value {
    let ranges: Vec<Value> = hazard
        .ranges
        .iter()
        .filter(|range| range.level > carte::Level::Green)
        .map(|range| {
            json!({
                "debut": range.begin.to_string(),
                "fin": range.end.to_string(),
                "niveau": range.level.name(),
            })
        })
        .collect();
    json!({
        "phenomene": hazard.phenomenon.name(),
        "niveau": hazard.level.name(),
        "littoral": hazard.coast,
        "plages": ranges,
    })
}

fn mark_unavailable(core: &Core) {
    for (entity, name) in [
        (TODAY_ENTITY, "Vigilance météo"),
        (TOMORROW_ENTITY, "Vigilance météo demain"),
    ] {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), name.into());
        set(core, entity, "unavailable", attributes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_metropolitan_departments_are_accepted() {
        for good in ["67", "01", "2a", " 2B ", "95", "29"] {
            assert!(Department::parse(good).is_ok(), "{good}");
        }
        for bad in ["20", "00", "96", "6", "067", "AB", ""] {
            assert!(Department::parse(bad).is_err(), "{bad}");
        }
        let error = Department::parse("974").unwrap_err().to_string();
        assert!(error.contains("outre-mer"), "{error}");
    }

    #[test]
    fn the_key_never_shows_up() {
        let config = VigilanceConfig::new("67", "  ma-cle-secrete\n", None).unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("ma-cle-secrete"), "{debug}");
        assert_eq!(config.key.as_str(), "ma-cle-secrete");
        assert_eq!(config.url.as_str(), DEFAULT_URL);
        for bad in ["", "   ", "avec espace", "accentuée", "a\u{0}b"] {
            assert!(VigilanceConfig::new("67", bad, None).is_err(), "{bad:?}");
        }
        assert!(VigilanceConfig::new("67", "k", Some("http://exemple.fr/")).is_err());
        assert!(VigilanceConfig::new("67", "k", Some("http://127.0.0.1:9/")).is_ok());
    }
}
