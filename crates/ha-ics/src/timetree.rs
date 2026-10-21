//! **TimeTree** (étape 74) : l'agenda partagé de la famille, en lecture seule.
//!
//! TimeTree ne publie ni fichier ICS ni API ouverte. On parle donc à
//! l'API de son site web (`timetreeapp.com/api/v1`), comme l'outil libre
//! TimeTree-Exporter :
//! 1. connexion avec l'adresse e-mail et le mot de passe d'un **compte dédié
//!    à la maison**, invité dans l'agenda partagé ; TimeTree rend un cookie
//!    de session `_session_id` ;
//! 2. la liste des agendas (`/calendars`), pour trouver le bon par son nom ;
//! 3. ses événements (`/calendar/<id>/events/sync`), par morceaux.
//!
//! Les événements deviennent un petit fichier ICS, lu par le même code que
//! les agendas en ligne : mêmes répétitions, mêmes fuseaux horaires.
//!
//! Ce n'est pas une API officielle : si TimeTree la change, l'agenda passe
//! « indisponible » avec la raison, et le reste de la maison continue.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;

use ha_storage::TimeTreeAccount;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::{Value, json};
use zeroize::Zeroizing;

/// L'adresse de l'API du site TimeTree.
pub const API: &str = "https://timetreeapp.com/api/v1";
/// L'en-tête que le site TimeTree envoie avec chaque requête.
const AGENT: &str = "web/2.1.0/fr";
/// Taille d'une réponse au plus.
const MAX_BYTES: usize = 2 << 20;
/// Morceaux d'événements lus au plus (chacun en compte des centaines).
const MAX_PAGES: usize = 50;
/// Événements gardés au plus.
const MAX_EVENTS: usize = 5000;

/// Ce qui peut mal se passer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeTreeError {
    #[error("adresse e-mail ou mot de passe TimeTree refusé")]
    Credentials,
    #[error("TimeTree demande d'attendre avant de se reconnecter")]
    RateLimited,
    #[error("TimeTree injoignable")]
    Unreachable,
    #[error("aucun agenda TimeTree ne s'appelle « {0} » (agendas : {1})")]
    NotFound(String, String),
    #[error("réponse inattendue de TimeTree ({0})")]
    Unexpected(String),
}

impl TimeTreeError {
    /// Faut-il attendre longtemps avant de réessayer (mot de passe refusé,
    /// trop de connexions) ?
    #[must_use]
    pub fn is_lasting(&self) -> bool {
        matches!(self, Self::Credentials | Self::RateLimited)
    }
}

/// Un agenda du compte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
    pub id: i64,
    pub name: String,
    /// Ses membres : identifiant, et nom affiché (pour les participants).
    pub users: Vec<(i64, String)>,
}

/// Une étiquette TimeTree (« Médecin »), et sa couleur `#rrggbb`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Label {
    pub name: String,
    pub color: String,
}

/// Ce qu'il faut pour habiller les rendez-vous : les étiquettes de l'agenda
/// (par numéro) et le nom de ses membres.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub labels: HashMap<i64, Label>,
    pub users: HashMap<i64, String>,
}

/// Le client : le compte, et la session en cours.
pub struct Client {
    http: reqwest::Client,
    base: String,
    account: TimeTreeAccount,
    session: Mutex<Option<Zeroizing<String>>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base", &self.base)
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// `base` : [`API`] (une autre adresse pour les tests).
    #[must_use]
    pub fn new(http: reqwest::Client, base: &str, account: TimeTreeAccount) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            account,
            session: Mutex::new(None),
        }
    }

    fn session(&self) -> Option<Zeroizing<String>> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn set_session(&self, session: Option<Zeroizing<String>>) {
        *self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = session;
    }

    /// Se connecte, et garde la session.
    pub async fn login(&self) -> Result<(), TimeTreeError> {
        let mut device = [0u8; 16];
        getrandom::fill(&mut device).map_err(|_| TimeTreeError::Unexpected("aléatoire".into()))?;
        let device: String = device.iter().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        });
        let body = Zeroizing::new(
            json!({
                "uid": self.account.email,
                "password": self.account.password.as_str(),
                "uuid": device,
            })
            .to_string(),
        );
        let response = self
            .http
            .put(format!("{}/auth/email/signin", self.base))
            .header("Content-Type", "application/json")
            .header("X-Timetreea", AGENT)
            .body(body.as_bytes().to_vec())
            .send()
            .await
            .map_err(|_| TimeTreeError::Unreachable)?;
        let status = response.status();
        let session = response
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(session_cookie);
        if !status.is_success() {
            let text = read(response).await.unwrap_or_default();
            let code = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|json| json.pointer("/error/code")?.as_i64());
            return Err(match code {
                Some(-702) => TimeTreeError::Credentials,
                Some(-495) => TimeTreeError::RateLimited,
                _ if status.as_u16() == 401 => TimeTreeError::Credentials,
                _ if status.as_u16() == 429 => TimeTreeError::RateLimited,
                _ => TimeTreeError::Unexpected(format!("connexion : {}", status.as_u16())),
            });
        }
        let session =
            session.ok_or_else(|| TimeTreeError::Unexpected("pas de session reçue".into()))?;
        self.set_session(Some(session));
        Ok(())
    }

    /// Une lecture, avec la session ; une session expirée est refaite une
    /// fois.
    async fn get(&self, path: &str) -> Result<Value, TimeTreeError> {
        for attempt in 0..2 {
            let session = if let Some(session) = self.session() {
                session
            } else {
                self.login().await?;
                self.session()
                    .ok_or_else(|| TimeTreeError::Unexpected("session".into()))?
            };
            let response = self
                .http
                .get(format!("{}{path}", self.base))
                .header("Content-Type", "application/json")
                .header("X-Timetreea", AGENT)
                .header("Cookie", format!("_session_id={}", session.as_str()))
                .send()
                .await
                .map_err(|_| TimeTreeError::Unreachable)?;
            let status = response.status().as_u16();
            if status == 401 && attempt == 0 {
                self.set_session(None);
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(TimeTreeError::Unexpected(format!("{path} : {status}")));
            }
            let text = read(response).await?;
            return serde_json::from_str(&text)
                .map_err(|_| TimeTreeError::Unexpected("réponse illisible".into()));
        }
        Err(TimeTreeError::Credentials)
    }

    /// Les agendas du compte (sans ceux qui ont été supprimés).
    pub async fn calendars(&self) -> Result<Vec<CalendarInfo>, TimeTreeError> {
        let json = self.get("/calendars?since=0").await?;
        let list = json
            .get("calendars")
            .and_then(Value::as_array)
            .ok_or_else(|| TimeTreeError::Unexpected("liste des agendas absente".into()))?;
        Ok(list
            .iter()
            .filter(|c| c.get("deactivated_at").is_none_or(Value::is_null))
            .filter_map(|c| {
                Some(CalendarInfo {
                    id: c.get("id")?.as_i64()?,
                    name: clean(c.get("name")?.as_str()?, 100),
                    users: c
                        .get("calendar_users")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|user| {
                            let id = user
                                .get("user_id")
                                .or_else(|| user.get("id"))
                                .and_then(Value::as_i64)?;
                            let name = clean(user.get("name")?.as_str()?, 40);
                            (!name.is_empty()).then_some((id, name))
                        })
                        .take(50)
                        .collect(),
                })
            })
            .take(100)
            .collect())
    }

    /// L'agenda qui s'appelle `name` (sans tenir compte des majuscules).
    pub async fn find(&self, name: &str) -> Result<CalendarInfo, TimeTreeError> {
        let calendars = self.calendars().await?;
        calendars
            .iter()
            .find(|c| c.name.trim().to_lowercase() == name.trim().to_lowercase())
            .cloned()
            .ok_or_else(|| {
                TimeTreeError::NotFound(
                    name.to_owned(),
                    calendars
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            })
    }

    /// Les étiquettes de l'agenda (« Médecin », rouge). Une erreur ici
    /// n'empêche pas de lire les rendez-vous : on s'en passe.
    pub async fn labels(&self, calendar_id: i64) -> HashMap<i64, Label> {
        let Ok(json) = self.get(&format!("/calendar/{calendar_id}/labels")).await else {
            return HashMap::new();
        };
        json.get("calendar_labels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|label| {
                let id = label.get("id").and_then(Value::as_i64)?;
                Some((
                    id,
                    Label {
                        name: clean(
                            label
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                            40,
                        ),
                        color: color(label.get("color")?).unwrap_or_default(),
                    },
                ))
            })
            .take(100)
            .collect()
    }

    /// Tous les événements de l'agenda, morceau par morceau.
    pub async fn events(&self, calendar_id: i64) -> Result<Vec<Value>, TimeTreeError> {
        let mut events = Vec::new();
        let mut since = 0i64;
        for _ in 0..MAX_PAGES {
            let json = self
                .get(&format!(
                    "/calendar/{calendar_id}/events/sync?since={since}"
                ))
                .await?;
            let page = json
                .get("events")
                .and_then(Value::as_array)
                .ok_or_else(|| TimeTreeError::Unexpected("événements absents".into()))?;
            events.extend(page.iter().cloned());
            if events.len() > MAX_EVENTS {
                events.truncate(MAX_EVENTS);
                break;
            }
            let more = json.get("chunk").and_then(Value::as_bool) == Some(true);
            match json.get("since").and_then(Value::as_i64) {
                Some(next) if more && next > since => since = next,
                _ => break,
            }
        }
        Ok(events)
    }
}

/// `_session_id=abc; path=/; HttpOnly` → `abc`.
fn session_cookie(header: &str) -> Option<Zeroizing<String>> {
    let value = header
        .split(';')
        .next()?
        .trim()
        .strip_prefix("_session_id=")?;
    let valid = (1..=512).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.%".contains(&b));
    valid.then(|| Zeroizing::new(value.to_owned()))
}

/// Lit une réponse, [`MAX_BYTES`] au plus.
async fn read(mut response: reqwest::Response) -> Result<String, TimeTreeError> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| TimeTreeError::Unreachable)?
    {
        if body.len() + chunk.len() > MAX_BYTES {
            return Err(TimeTreeError::Unexpected("réponse trop grosse".into()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Une couleur TimeTree : un nombre (`0xeb6834`) ou un texte `#eb6834`.
fn color(value: &Value) -> Option<String> {
    let text = match value {
        Value::Number(n) => format!("#{:06x}", n.as_u64().filter(|n| *n <= 0xff_ffff)?),
        Value::String(s) => s.trim().to_ascii_lowercase(),
        _ => return None,
    };
    crate::ics::is_hex_color(&text).then_some(text)
}

/// Le numéro d'étiquette d'un rendez-vous : `label_id`, ou dans
/// `relationships.label.data.id` (« 12345,3 » : le numéro est à la fin).
fn label_id(event: &Value) -> Option<i64> {
    if let Some(id) = event.get("label_id").and_then(Value::as_i64) {
        return Some(id);
    }
    let id = event.pointer("/relationships/label/data/id")?;
    match id {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.rsplit(',').next()?.trim().parse().ok(),
        _ => None,
    }
}

/// Les participants d'un rendez-vous, par leur nom (ceux qu'on connaît).
fn attendees(event: &Value, users: &HashMap<i64, String>) -> Vec<String> {
    event
        .get("attendees")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let id = a.as_i64().or_else(|| {
                a.get("user_id")
                    .or_else(|| a.get("id"))
                    .and_then(Value::as_i64)
            })?;
            users.get(&id).cloned()
        })
        .take(20)
        .collect()
}

fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Échappe un texte pour une ligne ICS.
fn ics_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().take(1000) {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Le moment `ms` (millisecondes depuis 1970) dans le fuseau `zone`.
fn moment(ms: i64, zone: &TimeZone) -> Option<jiff::civil::DateTime> {
    Some(
        Timestamp::from_millisecond(ms)
            .ok()?
            .to_zoned(zone.clone())
            .datetime(),
    )
}

/// Les événements TimeTree, en fichier ICS, habillés de leurs étiquettes et
/// participants (`details`).
///
/// On saute les anniversaires (générés par TimeTree à partir des
/// profils), les « mémos » (des notes sans date) et les événements
/// supprimés.
#[must_use]
pub fn to_ics(events: &[Value], details: &Details) -> String {
    let mut out =
        String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//ha-maison//TimeTree//FR\r\n");
    for event in events {
        let number = |key: &str| event.get(key).and_then(Value::as_i64);
        let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or_default();
        let birthday = number("type") == Some(1);
        let memo = number("category") == Some(2);
        let deleted = event.get("deactivated_at").is_some_and(|v| !v.is_null());
        if birthday || memo || deleted {
            continue;
        }
        let (Some(start), Some(end)) = (number("start_at"), number("end_at")) else {
            continue;
        };
        let uid: String = text("uuid")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(64)
            .collect();
        if uid.is_empty() {
            continue;
        }
        let zone_name = event
            .get("start_timezone")
            .and_then(Value::as_str)
            .unwrap_or("UTC");
        let (zone, known) = match TimeZone::get(zone_name) {
            Ok(zone) => (zone, true),
            Err(_) => (TimeZone::UTC, false),
        };
        let all_day = event.get("all_day").and_then(Value::as_bool) == Some(true);
        let (Some(begin), Some(finish)) = (moment(start, &zone), moment(end, &zone)) else {
            continue;
        };
        let _ = write!(
            out,
            "BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:{}\r\n",
            ics_text(text("title"))
        );
        let note = text("note");
        if !note.is_empty() {
            let _ = write!(out, "DESCRIPTION:{}\r\n", ics_text(note));
        }
        let location = text("location");
        if !location.is_empty() {
            let _ = write!(out, "LOCATION:{}\r\n", ics_text(location));
        }
        if let Some(label) = label_id(event).and_then(|id| details.labels.get(&id)) {
            if !label.name.is_empty() {
                let _ = write!(out, "CATEGORIES:{}\r\n", ics_text(&label.name));
            }
            if !label.color.is_empty() {
                let _ = write!(out, "COLOR:{}\r\n", label.color);
            }
        }
        for name in attendees(event, &details.users) {
            // Le nom entre guillemets : sans « " », sans retour à la ligne.
            let name: String = name
                .chars()
                .filter(|c| *c != '"' && !c.is_control())
                .collect();
            let _ = write!(out, "ATTENDEE;CN=\"{name}\":invalid:nomail\r\n");
        }
        if all_day {
            // La fin d'une journée entière est exclue en ICS : le lendemain.
            let last = finish.date().tomorrow().unwrap_or(finish.date());
            let _ = write!(
                out,
                "DTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\n",
                begin.strftime("%Y%m%d"),
                last.strftime("%Y%m%d")
            );
        } else if known && zone_name != "UTC" {
            let _ = write!(
                out,
                "DTSTART;TZID={zone_name}:{}\r\nDTEND;TZID={zone_name}:{}\r\n",
                begin.strftime("%Y%m%dT%H%M%S"),
                finish.strftime("%Y%m%dT%H%M%S")
            );
        } else {
            let _ = write!(
                out,
                "DTSTART:{}Z\r\nDTEND:{}Z\r\n",
                begin.strftime("%Y%m%dT%H%M%S"),
                finish.strftime("%Y%m%dT%H%M%S")
            );
        }
        for line in event
            .get("recurrences")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(20)
        {
            if let Some(line) = recurrence(line, all_day) {
                let _ = write!(out, "{line}\r\n");
            }
        }
        out.push_str("END:VEVENT\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}

/// Une ligne `RRULE:…` ou `EXDATE…` de TimeTree, vérifiée. Un `UNTIL` sans
/// heure sur un événement à heure fixe vaut jusqu'à la fin de ce jour-là.
fn recurrence(line: &str, all_day: bool) -> Option<String> {
    let line = line.trim();
    if line.len() > 500 || line.chars().any(char::is_control) {
        return None;
    }
    let upper = line.to_ascii_uppercase();
    if let Some(rule) = line.strip_prefix("RRULE:") {
        let parts: Vec<String> = rule
            .split(';')
            .map(|part| match part.split_once('=') {
                Some(("UNTIL", value))
                    if !all_day
                        && value.len() == 8
                        && value.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    format!("UNTIL={value}T235959")
                }
                _ => part.to_owned(),
            })
            .collect();
        return Some(format!("RRULE:{}", parts.join(";")));
    }
    (upper.starts_with("EXDATE") || upper.starts_with("RDATE")).then(|| line.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ics;
    use jiff::civil::date;

    fn paris() -> TimeZone {
        TimeZone::get("Europe/Paris").unwrap_or(TimeZone::UTC)
    }

    /// Le 21 octobre 2026 à `h` heures, heure de Paris (UTC+2), en ms.
    fn oct21(h: i64) -> i64 {
        1_792_533_600_000 + h * 3_600_000
    }

    #[test]
    fn events_become_an_ics_file() {
        let events = vec![
            json!({"uuid": "a1", "title": "Dentiste, Léa", "note": "Ligne 1\nLigne 2",
                   "location": "Cabinet; 1er étage", "attendees": [7, {"user_id": 8}, 99],
                   "relationships": {"label": {"data": {"id": "123,3"}}},
                   "start_at": oct21(15), "end_at": oct21(16), "all_day": false,
                   "start_timezone": "Europe/Paris", "type": 0, "category": 1,
                   "recurrences": []}),
            json!({"uuid": "b2", "title": "Piscine", "start_at": oct21(18), "end_at": oct21(19),
                   "all_day": false, "start_timezone": "Europe/Paris", "type": 0, "category": 1,
                   "recurrences": ["RRULE:FREQ=WEEKLY;UNTIL=20261104", "EXDATE;TZID=Europe/Paris:20261028T180000"]}),
            json!({"uuid": "c3", "title": "Vacances", "start_at": oct21(0), "end_at": oct21(0) + 2 * 86_400_000,
                   "all_day": true, "start_timezone": "Europe/Paris", "type": 0, "category": 1}),
            json!({"uuid": "d4", "title": "Anniversaire de Tom", "start_at": oct21(0), "end_at": oct21(0),
                   "all_day": true, "type": 1, "category": 1}),
            json!({"uuid": "e5", "title": "Idées cadeaux", "start_at": oct21(0), "end_at": oct21(0),
                   "all_day": true, "type": 0, "category": 2}),
            json!({"uuid": "f6", "title": "Supprimé", "start_at": oct21(9), "end_at": oct21(10),
                   "all_day": false, "type": 0, "category": 1, "deactivated_at": 1}),
            json!({"title": "Sans identifiant", "start_at": oct21(9), "end_at": oct21(10)}),
        ];
        let details = Details {
            labels: HashMap::from([(
                3,
                Label {
                    name: "Médecin".into(),
                    color: "#d6311f".into(),
                },
            )]),
            users: HashMap::from([(7, "Papa".to_owned()), (8, "Léa".to_owned())]),
        };
        let text = to_ics(&events, &details);
        assert!(text.contains("SUMMARY:Dentiste\\, Léa\r\n"), "{text}");
        assert!(
            text.contains(
                "LOCATION:Cabinet\\; 1er étage\r\nCATEGORIES:Médecin\r\nCOLOR:#d6311f\r\n"
            ),
            "{text}"
        );
        assert!(text.contains("ATTENDEE;CN=\"Papa\":invalid:nomail\r\n"));
        assert!(text.contains("DESCRIPTION:Ligne 1\\nLigne 2\r\n"));
        assert!(text.contains("DTSTART;TZID=Europe/Paris:20261021T150000\r\n"));
        assert!(text.contains("RRULE:FREQ=WEEKLY;UNTIL=20261104T235959\r\n"));
        assert!(text.contains("DTSTART;VALUE=DATE:20261021\r\nDTEND;VALUE=DATE:20261024\r\n"));
        for skipped in ["Anniversaire", "Idées", "Supprimé", "Sans identifiant"] {
            assert!(!text.contains(skipped), "{skipped}");
        }

        // Lu par le même code que les agendas en ligne.
        let parsed = ics::parse(&text, &paris());
        assert_eq!(parsed.events.len(), 3);
        let found = ics::expand(
            &parsed.events,
            date(2026, 10, 20).at(0, 0, 0, 0),
            date(2026, 11, 30).at(0, 0, 0, 0),
            &paris(),
        );
        let piscine: Vec<String> = found
            .iter()
            .filter(|o| o.summary == "Piscine")
            .map(|o| o.start.to_string())
            .collect();
        // Le 21, le 4 novembre (dernier jour inclus) ; pas le 28 (annulé).
        assert_eq!(piscine, ["2026-10-21T18:00:00", "2026-11-04T18:00:00"]);
        let dentiste = found.iter().find(|o| o.summary == "Dentiste, Léa").unwrap();
        assert_eq!(dentiste.extra.location, "Cabinet; 1er étage");
        assert_eq!(dentiste.extra.label, "Médecin");
        assert_eq!(dentiste.extra.color, "#d6311f");
        assert_eq!(dentiste.extra.attendees, ["Papa", "Léa"]);
        let vacances = found
            .iter()
            .find(|o| o.summary == "Vacances")
            .map(|o| (o.all_day, o.end.date()));
        assert_eq!(vacances, Some((true, date(2026, 10, 24))));
    }

    #[test]
    fn session_cookies_and_odd_recurrences() {
        assert_eq!(
            session_cookie("_session_id=abc123; path=/; HttpOnly")
                .as_deref()
                .map(String::as_str),
            Some("abc123")
        );
        assert!(session_cookie("autre=1; path=/").is_none());
        assert!(session_cookie("_session_id=a b").is_none());
        assert_eq!(
            recurrence("RRULE:FREQ=DAILY;COUNT=3", false).as_deref(),
            Some("RRULE:FREQ=DAILY;COUNT=3")
        );
        assert_eq!(
            recurrence("RRULE:FREQ=DAILY;UNTIL=20261104", true).as_deref(),
            Some("RRULE:FREQ=DAILY;UNTIL=20261104")
        );
        assert!(recurrence("X-AUTRE:1", false).is_none());
        assert!(recurrence("RRULE:FREQ=DAILY\r\nBEGIN:VEVENT", false).is_none());
        assert!(TimeTreeError::Credentials.is_lasting());
        assert_eq!(color(&json!(0xeb_6834)).as_deref(), Some("#eb6834"));
        assert_eq!(color(&json!("#1BAF7A")).as_deref(), Some("#1baf7a"));
        assert_eq!(color(&json!("rouge")), None);
        assert_eq!(label_id(&json!({"label_id": 4})), Some(4));
        assert!(!TimeTreeError::Unreachable.is_lasting());
    }
}
