//! **Météo locale**, avec Open-Meteo (<https://open-meteo.com>) : gratuit,
//! sans compte ni clé, données sous licence libre (CC BY 4.0).
//!
//! Comme dans Home Assistant, on obtient :
//! - `weather.maison` : la condition (`sunny`, `rainy`…) et, en attributs, la
//!   température, l'humidité, le vent… et les prévisions des 5 prochains jours ;
//! - des capteurs utilisables dans les automatisations et les graphiques :
//!   `sensor.meteo_temperature`, `sensor.meteo_humidite`, `sensor.meteo_vent`,
//!   `sensor.meteo_pluie`.
//!
//! Vie privée : la position de la maison est **arrondie à 0,1°** (environ
//! 10 km) avant d'être envoyée : assez pour la météo, trop flou pour te
//! situer. Elle n'est jamais écrite dans les logs, même dans un message d'erreur.

pub mod carte;
pub mod openmeteo;
pub mod vigilance;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId};
use ha_sun::Location;
use serde_json::{Value, json};
use tracing::{info, warn};

use crate::openmeteo::Report;

/// L'adresse d'Open-Meteo.
pub const DEFAULT_URL: &str = "https://api.open-meteo.com/v1/forecast";
/// Les mesures d'Open-Meteo changent tous les quarts d'heure.
pub const DEFAULT_INTERVAL: Duration = Duration::from_mins(15);
/// Après un échec, nouvel essai plus tôt.
const RETRY_DELAY: Duration = Duration::from_mins(5);
/// Par défaut, après 2 h sans nouvelles, les entités passent « indisponibles ».
const STALE_AFTER: Duration = Duration::from_hours(2);
/// Taille maximale d'une réponse (une vraie fait ~2 Kio).
const MAX_RESPONSE: usize = 256 * 1024;

pub const WEATHER_ENTITY: &str = "weather.maison";
const ATTRIBUTION: &str = "Données météo : Open-Meteo.com (CC BY 4.0)";

/// Les capteurs : (entité, nom, unité, classe d'appareil).
const SENSORS: [(&str, &str, &str, &str); 4] = [
    (
        "sensor.meteo_temperature",
        "Température extérieure",
        "°C",
        "temperature",
    ),
    (
        "sensor.meteo_humidite",
        "Humidité extérieure",
        "%",
        "humidity",
    ),
    ("sensor.meteo_vent", "Vent", "km/h", "wind_speed"),
    ("sensor.meteo_pluie", "Pluie", "mm", "precipitation"),
];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WeatherError {
    #[error("adresse du service météo invalide : {0}")]
    Url(String),
    #[error("vigilance : {0}")]
    Vigilance(String),
}

/// Réglages.
#[derive(Debug, Clone)]
pub struct Config {
    url: url::Url,
    latitude: f64,
    longitude: f64,
    pub interval: Duration,
    /// Sans nouvelles depuis aussi longtemps, les entités passent « indisponibles ».
    pub stale_after: Duration,
}

impl Config {
    /// `url` : `None` pour Open-Meteo ; sinon un serveur Open-Meteo
    /// auto-hébergé (HTTP accepté seulement sur le réseau local).
    pub fn new(location: Location, url: Option<&str>) -> Result<Self, WeatherError> {
        let url = check_url(url.unwrap_or(DEFAULT_URL))?;
        Ok(Self {
            url,
            latitude: round(location.latitude()),
            longitude: round(location.longitude()),
            interval: DEFAULT_INTERVAL,
            stale_after: STALE_AFTER,
        })
    }

    /// L'adresse appelée, avec la position arrondie.
    #[must_use]
    pub fn request_url(&self) -> url::Url {
        openmeteo::request_url(&self.url, self.latitude, self.longitude)
    }

    /// Le serveur appelé, **sans** la position : pour les logs.
    #[must_use]
    pub fn server(&self) -> String {
        self.url.host_str().unwrap_or("?").to_owned()
    }
}

/// Vérifie l'adresse d'un service : HTTPS, ou HTTP sur le réseau local ;
/// sans identifiants, `?` ni `#`.
pub(crate) fn check_url(raw: &str) -> Result<url::Url, WeatherError> {
    let url = url::Url::parse(raw.trim()).map_err(|error| WeatherError::Url(error.to_string()))?;
    match url.scheme() {
        "https" => {}
        "http" if is_local(&url) => {}
        "http" => {
            return Err(WeatherError::Url(
                "HTTP non chiffré seulement vers le réseau local ; utilise https://".into(),
            ));
        }
        other => return Err(WeatherError::Url(format!("protocole {other:?} refusé"))),
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WeatherError::Url(
            "pas d'identifiants, de « ? » ni de « # » dans l'adresse".into(),
        ));
    }
    Ok(url)
}

/// Arrondi à 0,1° (environ 10 km).
fn round(degrees: f64) -> f64 {
    (degrees * 10.0).round() / 10.0
}

/// Adresse du réseau local : `localhost`, IP privée, nom en `.local`/`.lan`.
fn is_local(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        Some(url::Host::Domain(name)) => {
            name == "localhost" || matches!(name.rsplit_once('.'), Some((_, "local" | "lan")))
        }
        None => false,
    }
}

/// Le service météo, en marche.
#[derive(Debug)]
pub struct Weather(Background);

impl Weather {
    pub fn start(config: Config, core: Arc<Core>) -> Result<Self, ha_tls::TlsError> {
        let http = ha_tls::web_client(config.url.scheme() == "https")?;
        Ok(Self(Background::spawn("la météo", move |token| {
            run(config, http, core, token)
        })))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(config: Config, http: reqwest::Client, core: Arc<Core>, token: CancellationToken) {
    info!(
        "météo : {} toutes les {} min (position arrondie à 10 km)",
        config.server(),
        config.interval.as_secs() / 60
    );
    let mut last_success: Option<Instant> = None;
    let started = Instant::now();
    let mut failing = false;
    loop {
        let delay = match fetch(&http, &config).await {
            Ok(report) => {
                if failing {
                    info!("météo : de nouveau disponible");
                }
                failing = false;
                last_success = Some(Instant::now());
                publish(&core, &report);
                config.interval
            }
            Err(error) => {
                // Un seul message tant que la panne dure : pas de logs inondés.
                if !failing {
                    warn!("météo : {error} (nouvel essai dans quelques minutes)");
                }
                failing = true;
                let since = last_success.unwrap_or(started);
                if since.elapsed() >= config.stale_after {
                    mark_unavailable(&core);
                }
                RETRY_DELAY.min(config.interval)
            }
        };
        tokio::select! {
            () = token.cancelled() => break,
            () = tokio::time::sleep(delay) => {}
        }
    }
}

/// Une requête à Open-Meteo.
async fn fetch(http: &reqwest::Client, config: &Config) -> Result<Report, String> {
    // Sécurité : le message d'une erreur `reqwest` contient l'adresse
    // appelée, donc la position. `without_url` la retire avant les logs.
    let describe = |error: reqwest::Error| error.without_url().to_string();
    let mut response = http
        .get(config.request_url())
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(describe)?;
    let status = response.status();
    // On lit la réponse morceau par morceau, en s'arrêtant si elle est trop
    // grosse : un serveur ne peut pas nous faire remplir la mémoire.
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(describe)? {
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err("réponse trop grosse".into());
        }
        body.extend_from_slice(&chunk);
    }
    match openmeteo::parse(&body) {
        Ok(report) if status.is_success() => Ok(report),
        // Open-Meteo explique ses refus (limite atteinte…) dans le corps.
        Err(reason) if status.is_success() || reason.starts_with("Open-Meteo") => Err(reason),
        _ => Err(format!("erreur HTTP {status}")),
    }
}

/// Met à jour `weather.maison` et les capteurs.
fn publish(core: &Core, report: &Report) {
    let c = &report.current;
    let forecast: Vec<Value> = report.days.iter().map(openmeteo::Day::to_json).collect();
    let mut attributes = object(json!({
        "friendly_name": "Météo",
        "temperature": c.temperature,
        "apparent_temperature": c.apparent_temperature,
        "humidity": c.humidity,
        "pressure": c.pressure,
        "wind_speed": c.wind_speed,
        "wind_gust_speed": c.wind_gust,
        "wind_bearing": c.wind_bearing,
        "cloud_coverage": c.cloud_cover,
        "precipitation": c.precipitation,
        "temperature_unit": "°C",
        "pressure_unit": "hPa",
        "wind_speed_unit": "km/h",
        "precipitation_unit": "mm",
        "forecast": forecast,
        "attribution": ATTRIBUTION,
    }));
    attributes.retain(|_, value| !value.is_null());
    set(core, WEATHER_ENTITY, c.condition, attributes);

    let values = [
        format!("{:.1}", c.temperature),
        format!("{:.0}", c.humidity),
        format!("{:.1}", c.wind_speed),
        format!("{:.1}", c.precipitation),
    ];
    for ((entity, name, unit, class), value) in SENSORS.iter().zip(values) {
        set(core, entity, &value, sensor_attributes(name, unit, class));
    }
}

fn sensor_attributes(name: &str, unit: &str, class: &str) -> Attributes {
    object(json!({
        "friendly_name": name,
        "unit_of_measurement": unit,
        "device_class": class,
        "state_class": "measurement",
        "attribution": ATTRIBUTION,
    }))
}

/// Plus de nouvelles depuis longtemps : on ne laisse pas croire que les
/// valeurs sont à jour.
fn mark_unavailable(core: &Core) {
    let mut weather = Attributes::new();
    weather.insert("friendly_name".into(), "Météo".into());
    set(core, WEATHER_ENTITY, "unavailable", weather);
    for (entity, name, unit, class) in SENSORS {
        set(
            core,
            entity,
            "unavailable",
            sensor_attributes(name, unit, class),
        );
    }
}

pub(crate) fn set(core: &Core, entity: &str, state: &str, attributes: Attributes) {
    let Ok(entity_id) = entity.parse::<EntityId>() else {
        return;
    };
    // Rien ne change : on n'écrit pas (l'historique reste léger).
    if let Some(current) = core.states.get(&entity_id)
        && current.state() == state
        && current.attributes() == &attributes
    {
        return;
    }
    if let Err(error) = core.states.set(entity_id, state, attributes) {
        warn!("météo : état refusé ({error})");
    }
}

pub(crate) fn object(value: Value) -> Attributes {
    match value {
        Value::Object(map) => map,
        _ => Attributes::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paris() -> Location {
        Location::new(48.8566, 2.3522).unwrap()
    }

    #[test]
    fn the_position_is_rounded_to_about_10_km() {
        let config = Config::new(paris(), None).unwrap();
        let url = config.request_url();
        let query = url.query().unwrap();
        assert!(query.contains("latitude=48.9&longitude=2.4"), "{query}");
        assert!(!query.contains("48.85"));
        assert_eq!(config.server(), "api.open-meteo.com");
        let south = Config::new(Location::new(-33.86, -151.21).unwrap(), None).unwrap();
        assert!(
            south
                .request_url()
                .query()
                .unwrap()
                .contains("latitude=-33.9&longitude=-151.2")
        );
    }

    #[tokio::test]
    async fn error_messages_never_reveal_the_position() {
        // Port fermé : la connexion échoue, et `reqwest` mettrait l'adresse
        // (donc la position) dans son message.
        let config = Config::new(paris(), Some("http://127.0.0.1:1/v1/forecast")).unwrap();
        let http = ha_tls::web_client(false).unwrap();
        let error = fetch(&http, &config).await.unwrap_err();
        assert!(
            !error.contains("latitude") && !error.contains("48.9"),
            "{error}"
        );
    }

    #[test]
    fn only_safe_addresses_are_accepted() {
        for good in [
            "https://api.open-meteo.com/v1/forecast",
            "http://192.168.1.10:8080/v1/forecast",
            "http://meteo.lan/v1/forecast",
        ] {
            assert!(Config::new(paris(), Some(good)).is_ok(), "{good}");
        }
        for bad in [
            "http://api.open-meteo.com/v1/forecast",
            "ftp://exemple.fr",
            "https://moi:secret@exemple.fr/",
            "https://exemple.fr/?latitude=1",
            "pas une adresse",
        ] {
            assert!(Config::new(paris(), Some(bad)).is_err(), "{bad}");
        }
    }
}
