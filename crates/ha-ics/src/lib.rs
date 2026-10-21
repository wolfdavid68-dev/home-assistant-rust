//! **Agendas en ligne** (étape 57), comme l'intégration « Remote Calendar »
//! de Home Assistant : un fichier ICS publié sur Internet (collecte des
//! déchets de la commune, vacances scolaires, agenda Google partagé…)
//! devient un agenda en lecture seule, `calendar.<nom>`.
//!
//! ```yaml
//! # calendriers.yaml
//! - nom: Collecte des déchets
//!   url: https://www.exemple.fr/collecte.ics
//! - nom: Vacances scolaires
//!   url: webcal://www.exemple.fr/zone-b.ics
//! ```
//!
//! L'agenda a le même état que celui de la maison (`on` pendant un
//! événement) et la même liste `events` : le déclencheur `calendar` des
//! automatisations marche tel quel (« la veille de la collecte… »).
//!
//! Sécurité :
//! - HTTPS seulement, sans redirection, 2 Mio au plus ;
//! - l'adresse contient souvent une clé privée (Google) : elle n'est
//!   jamais écrite dans les logs ni dans l'état de l'agenda, seulement le
//!   nom du site ;
//! - relu toutes les 6 heures (15 minutes après un échec).
//!
//! Un agenda **TimeTree** (étape 74) se déclare de la même façon, avec son
//! nom dans TimeTree au lieu d'une adresse (voir [`timetree`]) :
//!
//! ```yaml
//! - nom: Famille
//!   timetree: Famille
//! ```

pub mod ics;
pub mod timetree;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId};
use ha_storage::{CalendarEvent, TimeTreeAccount};
use jiff::ToSpan;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

/// Agendas en ligne au plus.
pub const MAX_CALENDARS: usize = 10;
/// Taille d'un fichier au plus.
pub const MAX_BYTES: usize = 2 << 20;
const REFRESH: Duration = Duration::from_hours(6);
const RETRY: Duration = Duration::from_mins(15);
/// L'agenda de la famille change souvent : relu toutes les 15 minutes.
const TIMETREE_REFRESH: Duration = Duration::from_mins(15);
/// Mot de passe refusé, trop de connexions : on attend une heure.
const TIMETREE_LASTING_RETRY: Duration = Duration::from_hours(1);
/// Les événements publiés : la semaine passée et les deux mois à venir.
const PAST_DAYS: i64 = 7;
const FUTURE_DAYS: i64 = 62;
/// Événements publiés au plus dans l'état de l'agenda.
const MAX_PUBLISHED: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("lecture de {path} impossible : {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

/// D'où vient l'agenda.
#[derive(Clone, PartialEq, Eq)]
pub enum Source {
    /// Un fichier ICS en ligne.
    Ics(url::Url),
    /// Un agenda TimeTree, par son nom.
    TimeTree(String),
}

/// Jamais l'adresse complète : elle peut contenir une clé.
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ics(url) => write!(f, "Ics({})", url.host_str().unwrap_or_default()),
            Self::TimeTree(name) => write!(f, "TimeTree({name})"),
        }
    }
}

impl Source {
    /// Ce qu'on peut afficher : le site (jamais l'adresse complète, qui
    /// peut contenir une clé), ou « TimeTree ».
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Ics(url) => url.host_str().unwrap_or_default().to_owned(),
            Self::TimeTree(_) => "TimeTree".to_owned(),
        }
    }
}

/// Un agenda en ligne.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteCalendar {
    pub name: String,
    pub entity_id: EntityId,
    pub source: Source,
    /// La couleur de l'agenda dans le tableau de bord (`#rrggbb`), si
    /// elle est choisie.
    pub color: Option<String>,
}

/// Pas d'adresse dans les affichages de débogage : elle peut contenir une
/// clé privée.
impl std::fmt::Debug for RemoteCalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteCalendar")
            .field("name", &self.name)
            .field("entity_id", &self.entity_id)
            .field("source", &self.source.label())
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    nom: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    timetree: Option<String>,
    #[serde(default)]
    couleur: Option<String>,
}

/// Vérifie le contenu de `calendriers.yaml`.
pub fn parse_config(text: &str) -> Result<Vec<RemoteCalendar>, ConfigError> {
    let entries: Vec<Entry> = if text.trim().is_empty() {
        Vec::new()
    } else {
        serde_saphyr::from_str(text).map_err(|error| ConfigError::Invalid(error.to_string()))?
    };
    if entries.len() > MAX_CALENDARS {
        return Err(ConfigError::Invalid(format!(
            "{MAX_CALENDARS} agendas en ligne au plus"
        )));
    }
    let mut calendars: Vec<RemoteCalendar> = Vec::new();
    for entry in entries {
        let name = entry.nom.trim().to_owned();
        if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
            return Err(ConfigError::Invalid(format!(
                "nom « {name} » : 1 à 60 caractères"
            )));
        }
        let slug = ha_core::slugify(&name);
        if slug.is_empty() || slug == "maison" {
            return Err(ConfigError::Invalid(format!(
                "nom « {name} » : choisis un autre nom (« maison » est l'agenda de la maison)"
            )));
        }
        let entity_id: EntityId = format!("calendar.{slug}")
            .parse()
            .map_err(|_| ConfigError::Invalid(format!("nom « {name} » invalide")))?;
        if calendars.iter().any(|c| c.entity_id == entity_id) {
            return Err(ConfigError::Invalid(format!(
                "deux agendas s'appellent « {name} »"
            )));
        }
        let source = match (entry.url, entry.timetree) {
            (Some(url), None) => Source::Ics(parse_url(&name, &url)?),
            (None, Some(calendar)) => {
                let calendar = calendar.trim().to_owned();
                if calendar.is_empty()
                    || calendar.chars().count() > 100
                    || calendar.chars().any(char::is_control)
                {
                    return Err(ConfigError::Invalid(format!(
                        "« {name} » : nom de l'agenda TimeTree invalide"
                    )));
                }
                Source::TimeTree(calendar)
            }
            _ => {
                return Err(ConfigError::Invalid(format!(
                    "« {name} » : indique soit url, soit timetree (le nom de l'agenda TimeTree)"
                )));
            }
        };
        let color = match entry.couleur.map(|c| c.trim().to_ascii_lowercase()) {
            None => None,
            Some(color) if ics::is_hex_color(&color) => Some(color),
            Some(color) => {
                return Err(ConfigError::Invalid(format!(
                    "« {name} » : couleur « {color} » invalide (exemple : \"#4caf50\")"
                )));
            }
        };
        calendars.push(RemoteCalendar {
            name,
            entity_id,
            source,
            color,
        });
    }
    Ok(calendars)
}

fn parse_url(name: &str, raw: &str) -> Result<url::Url, ConfigError> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("webcal://")
        .map_or_else(|| raw.to_owned(), |rest| format!("https://{rest}"));
    url::Url::parse(&raw)
        .ok()
        .filter(|url| url.scheme() == "https" && url.host_str().is_some())
        .filter(|url| url.username().is_empty() && url.password().is_none())
        .ok_or_else(|| {
            ConfigError::Invalid(format!(
                "« {name} » : adresse invalide (https:// ou webcal://, sans identifiants)"
            ))
        })
}

/// Lit `calendriers.yaml` ; un fichier absent ne donne aucun agenda.
pub fn load(path: &Path) -> Result<Vec<RemoteCalendar>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_config(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(ConfigError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Un identifiant stable pour une occurrence (la même à chaque relecture).
fn occurrence_id(occurrence: &ics::Occurrence) -> i64 {
    // FNV-1a : simple et stable d'une version à l'autre.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in occurrence
        .uid
        .bytes()
        .chain(occurrence.start.to_string().into_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    i64::try_from(hash >> 1).unwrap_or(0)
}

/// Les événements à publier, au format de l'agenda de la maison.
#[must_use]
pub fn window(events: &[ics::Event], now: DateTime, local: &TimeZone) -> Vec<CalendarEvent> {
    let from = now.checked_sub(PAST_DAYS.days()).unwrap_or(now);
    let to = now.checked_add(FUTURE_DAYS.days()).unwrap_or(now);
    ics::expand(events, from, to, local)
        .into_iter()
        .take(MAX_PUBLISHED)
        .map(|occurrence| CalendarEvent {
            id: occurrence_id(&occurrence),
            summary: occurrence.summary.chars().take(200).collect(),
            description: occurrence.description.chars().take(1000).collect(),
            start: occurrence.start.strftime("%Y-%m-%dT%H:%M:%S").to_string(),
            end: occurrence.end.strftime("%Y-%m-%dT%H:%M:%S").to_string(),
            all_day: occurrence.all_day,
            created_by: None,
            extra: occurrence.extra,
        })
        .collect()
}

/// L'état de l'agenda : celui de l'agenda de la maison, en lecture seule.
#[must_use]
pub fn calendar_state(
    calendar: &RemoteCalendar,
    events: Option<&[ics::Event]>,
    error: Option<&str>,
    now: DateTime,
    local: &TimeZone,
) -> (String, Attributes) {
    let Some(events) = events else {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), json!(calendar.name));
        attributes.insert("read_only".into(), json!(true));
        if let Some(error) = error {
            attributes.insert("erreur".into(), json!(error));
        }
        return ("unavailable".into(), attributes);
    };
    let (state, mut attributes) =
        ha_agenda::calendar_state(&calendar.name, &window(events, now, local), now);
    attributes.insert("read_only".into(), json!(true));
    attributes.insert("source".into(), json!(calendar.source.label()));
    if let Some(color) = &calendar.color {
        attributes.insert("couleur".into(), json!(color));
    }
    if let Some(error) = error {
        attributes.insert("erreur".into(), json!(error));
    }
    (state, attributes)
}

/// Télécharge le fichier (2 Mio au plus).
async fn fetch(http: &reqwest::Client, url: &url::Url) -> Result<String, String> {
    let mut response = http
        .get(url.clone())
        .header("Accept", "text/calendar")
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "le site ne répond pas".to_owned()
            } else {
                "site injoignable".to_owned()
            }
        })?;
    if !response.status().is_success() {
        return Err(format!("le site a répondu {}", response.status().as_u16()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "téléchargement interrompu".to_owned())?
    {
        if body.len() + chunk.len() > MAX_BYTES {
            return Err("fichier trop gros (2 Mio au plus)".into());
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    if !text.contains("BEGIN:VCALENDAR") {
        return Err("ce n'est pas un agenda (ICS)".into());
    }
    Ok(text)
}

struct Slot {
    calendar: RemoteCalendar,
    events: Option<Vec<ics::Event>>,
    error: Option<String>,
    next_fetch: Instant,
    /// L'agenda TimeTree trouvé par son nom.
    timetree_id: Option<timetree::CalendarInfo>,
}

/// Le service : relit les agendas et publie leur état chaque minute.
#[derive(Debug)]
pub struct Calendars(Background);

impl Calendars {
    /// `timetree` : le compte TimeTree de la maison, s'il y en a un.
    pub fn start(
        calendars: Vec<RemoteCalendar>,
        timetree: Option<TimeTreeAccount>,
        core: Arc<Core>,
    ) -> Result<Self, ha_tls::TlsError> {
        Self::start_with(calendars, timetree, timetree::API, core)
    }

    /// Comme [`Calendars::start`], avec l'adresse de l'API TimeTree (pour
    /// les tests).
    pub fn start_with(
        calendars: Vec<RemoteCalendar>,
        timetree: Option<TimeTreeAccount>,
        timetree_api: &str,
        core: Arc<Core>,
    ) -> Result<Self, ha_tls::TlsError> {
        let http = ha_tls::public_client()?;
        // L'API de test est en HTTP, sur ce PC.
        let timetree_http = if timetree_api.starts_with("http://127.0.0.1") {
            ha_tls::web_client(false)?
        } else {
            http.clone()
        };
        let timetree = timetree
            .map(|account| Arc::new(timetree::Client::new(timetree_http, timetree_api, account)));
        Ok(Self(Background::spawn(
            "les agendas en ligne",
            move |token| run(calendars, timetree, core, http, token),
        )))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

/// Lit un agenda : le fichier ICS, ou les événements TimeTree mis en ICS.
/// Rend aussi le délai avant la prochaine lecture.
async fn read_source(
    source: &Source,
    http: &reqwest::Client,
    timetree: Option<&timetree::Client>,
    found: &mut Option<timetree::CalendarInfo>,
) -> Result<(String, Duration), (String, Duration)> {
    match source {
        Source::Ics(url) => fetch(http, url)
            .await
            .map(|text| (text, REFRESH))
            .map_err(|error| (error, RETRY)),
        Source::TimeTree(name) => {
            let Some(client) = timetree else {
                return Err((
                    "compte TimeTree non associé (ha-server timetree pair)".into(),
                    TIMETREE_LASTING_RETRY,
                ));
            };
            let wait = |error: &timetree::TimeTreeError| {
                if error.is_lasting() {
                    TIMETREE_LASTING_RETRY
                } else {
                    RETRY
                }
            };
            let calendar = if let Some(calendar) = found.clone() {
                calendar
            } else {
                let calendar = client
                    .find(name)
                    .await
                    .map_err(|e| (e.to_string(), wait(&e)))?;
                *found = Some(calendar.clone());
                calendar
            };
            match client.events(calendar.id).await {
                Ok(events) => {
                    // Les étiquettes peuvent changer : relues à chaque fois.
                    let details = timetree::Details {
                        labels: client.labels(calendar.id).await,
                        users: calendar.users.iter().cloned().collect(),
                    };
                    Ok((timetree::to_ics(&events, &details), TIMETREE_REFRESH))
                }
                Err(error) => {
                    // L'agenda a peut-être été recréé : on le recherchera.
                    *found = None;
                    Err((error.to_string(), wait(&error)))
                }
            }
        }
    }
}

async fn run(
    calendars: Vec<RemoteCalendar>,
    timetree: Option<Arc<timetree::Client>>,
    core: Arc<Core>,
    http: reqwest::Client,
    token: CancellationToken,
) {
    let local = TimeZone::system();
    let mut slots: Vec<Slot> = calendars
        .into_iter()
        .map(|calendar| Slot {
            calendar,
            events: None,
            error: None,
            next_fetch: Instant::now(),
            timetree_id: None,
        })
        .collect();
    info!("agendas en ligne : {} agenda(s)", slots.len());
    let mut ticker = tokio::time::interval(Duration::from_mins(1));
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {}
        }
        for slot in &mut slots {
            if Instant::now() >= slot.next_fetch {
                let host = slot.calendar.source.label();
                // L'adresse n'est jamais écrite : seulement le site.
                match read_source(
                    &slot.calendar.source,
                    &http,
                    timetree.as_deref(),
                    &mut slot.timetree_id,
                )
                .await
                {
                    Ok((text, refresh)) => {
                        let parsed = ics::parse(&text, &local);
                        info!(
                            "agenda « {} » ({host}) : {} événement(s){}",
                            slot.calendar.name,
                            parsed.events.len(),
                            if parsed.simplified > 0 {
                                format!(", {} répétition(s) trop compliquée(s)", parsed.simplified)
                            } else {
                                String::new()
                            }
                        );
                        slot.events = Some(parsed.events);
                        slot.error = None;
                        slot.next_fetch = Instant::now() + refresh;
                    }
                    Err((error, retry)) => {
                        warn!("agenda « {} » ({host}) : {error}", slot.calendar.name);
                        slot.error = Some(error);
                        slot.next_fetch = Instant::now() + retry;
                    }
                }
            }
            let now = jiff::Zoned::now().with_time_zone(local.clone()).datetime();
            let (state, attributes) = calendar_state(
                &slot.calendar,
                slot.events.as_deref(),
                slot.error.as_deref(),
                now,
                &local,
            );
            if let Err(error) = core
                .states
                .set(slot.calendar.entity_id.clone(), state, attributes)
            {
                warn!("agenda « {} » : état refusé ({error})", slot.calendar.name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use jiff::civil::date;

    #[test]
    fn the_config_is_checked() {
        let calendars = parse_config(
            "- nom: Collecte des déchets\n  url: https://www.exemple.fr/collecte.ics?cle=secret\n\
             - nom: Vacances scolaires\n  url: webcal://exemple.fr/zone-b.ics\n",
        )
        .unwrap();
        assert_eq!(calendars.len(), 2);
        assert_eq!(
            calendars[0].entity_id.as_str(),
            "calendar.collecte_des_dechets"
        );
        assert_eq!(
            calendars[1].source,
            Source::Ics(url::Url::parse("https://exemple.fr/zone-b.ics").unwrap())
        );
        let family = parse_config(
            "- nom: Famille\n  timetree: Agenda de la famille\n  couleur: \"#4CAF50\"\n",
        )
        .unwrap();
        assert_eq!(family[0].color.as_deref(), Some("#4caf50"));
        assert!(parse_config("- nom: X\n  timetree: X\n  couleur: vert\n").is_err());
        assert_eq!(
            family[0].source,
            Source::TimeTree("Agenda de la famille".into())
        );
        assert_eq!(family[0].entity_id.as_str(), "calendar.famille");
        assert!(
            !format!("{calendars:?}").contains("secret"),
            "pas d'adresse affichée"
        );
        assert_eq!(parse_config("").unwrap().len(), 0);
        for bad in [
            "- nom: X\n  url: http://exemple.fr/a.ics\n",
            "- nom: X\n  url: https://moi:mdp@exemple.fr/a.ics\n",
            "- nom: X\n  url: file:///etc/passwd\n",
            "- nom: Maison\n  url: https://exemple.fr/a.ics\n",
            "- nom: X\n  url: https://a.fr/1.ics\n- nom: x\n  url: https://a.fr/2.ics\n",
            "- nom: X\n  url: https://a.fr/1.ics\n  couleur: bleu\n",
            "- nom: X\n",
            "- nom: X\n  url: https://a.fr/1.ics\n  timetree: X\n",
            "- nom: X\n  timetree: \"\"\n",
        ] {
            assert!(parse_config(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_example_file_is_understood() {
        let calendars = parse_config(include_str!("../../../calendriers.example.yaml")).unwrap();
        assert_eq!(calendars.len(), 1);
        assert_eq!(
            calendars[0].entity_id.as_str(),
            "calendar.vacances_scolaires"
        );
    }

    #[test]
    fn the_state_matches_the_house_agenda() {
        let calendar = parse_config("- nom: Collecte\n  url: https://exemple.fr/c.ics\n")
            .unwrap()
            .remove(0);
        let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:om\r\nSUMMARY:Ordures ménagères\r\n\
                    DTSTART;VALUE=DATE:20261006\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let parsed = ics::parse(text, &TimeZone::UTC);
        let (state, attributes) = calendar_state(
            &calendar,
            Some(&parsed.events),
            None,
            date(2026, 10, 20).at(9, 0, 0, 0),
            &TimeZone::UTC,
        );
        assert_eq!(state, "on", "un mardi de collecte");
        assert_eq!(attributes["message"], "Ordures ménagères");
        assert_eq!(attributes["read_only"], true);
        assert_eq!(attributes["source"], "exemple.fr");
        let events = attributes["events"].as_array().unwrap();
        // Du mardi 13 octobre (une semaine avant) au mardi 15 décembre (le
        // 22 est au-delà des deux mois) : 10 mardis.
        assert_eq!(events.first().unwrap()["start"], "2026-10-13T00:00:00");
        assert_eq!(events.last().unwrap()["start"], "2026-12-15T00:00:00");
        assert_eq!(events.len(), 10);
        assert_eq!(events[0]["all_day"], true);
        // Les identifiants restent les mêmes d'une relecture à l'autre.
        let again = window(
            &parsed.events,
            date(2026, 10, 20).at(9, 0, 0, 0),
            &TimeZone::UTC,
        );
        assert_eq!(again[0].id.to_string(), events[0]["uid"]);

        let (state, attributes) = calendar_state(
            &calendar,
            None,
            Some("site injoignable"),
            date(2026, 10, 20).at(9, 0, 0, 0),
            &TimeZone::UTC,
        );
        assert_eq!(state, "unavailable");
        assert_eq!(attributes["erreur"], "site injoignable");
    }
}
