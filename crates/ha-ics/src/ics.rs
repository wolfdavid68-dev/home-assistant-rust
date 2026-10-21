//! Lire un fichier ICS (RFC 5545) : les événements, leurs heures, et les
//! répétitions les plus courantes (`RRULE`).
//!
//! Ce qui est compris : `DTSTART`/`DTEND`/`DURATION` (journée entière, UTC,
//! `TZID` ou heure locale), `SUMMARY`, `DESCRIPTION`, `UID`, `STATUS`
//! (annulé : ignoré), `EXDATE`, `RECURRENCE-ID` (une occurrence modifiée),
//! et `RRULE` avec `FREQ` (DAILY, WEEKLY, MONTHLY, YEARLY), `INTERVAL`,
//! `COUNT`, `UNTIL`, `BYDAY` (« 2TU » : le deuxième mardi) et `BYMONTHDAY`.
//! Une répétition plus compliquée (`BYMONTH`, `BYSETPOS`…) ne garde que sa
//! première date : mieux vaut un rendez-vous de moins qu'un faux.

use std::collections::HashSet;

use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, ToSpan};

pub use ha_storage::EventExtra;

/// Événements gardés au plus par fichier.
pub const MAX_EVENTS: usize = 5000;
/// Périodes calculées au plus par événement répété.
const MAX_STEPS: i64 = 4000;

/// Les paramètres d'une ligne (`TZID=Europe/Paris`).
type Params = Vec<(String, String)>;
/// Une ligne : nom, paramètres, valeur.
type Property = (String, Params, String);

/// Ce qu'un `VEVENT` donne.
enum Read {
    Event(Box<Event>),
    Cancelled,
}

/// Un événement lu dans le fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub description: String,
    /// En heure locale.
    pub start: DateTime,
    pub end: DateTime,
    pub all_day: bool,
    /// Le début et la fin dans le fuseau du fichier (UTC, `TZID`…) : les
    /// répétitions se calculent là, puis passent en heure locale (sinon une
    /// répétition en UTC prendrait une heure de travers à l'heure d'été).
    pub source_start: DateTime,
    pub source_end: DateTime,
    /// `None` : heure « flottante » ou journée entière, déjà locale.
    pub zone: Option<TimeZone>,
    pub rule: Option<Rule>,
    pub exdates: Vec<DateTime>,
    /// L'occurrence d'un événement répété que celui-ci remplace.
    pub recurrence_id: Option<DateTime>,
    /// Lieu, étiquette, couleur, participants.
    pub extra: EventExtra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// Une règle de répétition (sous-ensemble de `RRULE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: i32,
    pub count: Option<usize>,
    pub until: Option<DateTime>,
    /// `BYDAY` : jour, et rang dans le mois (« 2TU », « -1FR »).
    pub by_day: Vec<(Option<i8>, Weekday)>,
    pub by_month_day: Vec<i8>,
}

/// Une occurrence, prête à afficher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub start: DateTime,
    pub end: DateTime,
    pub all_day: bool,
    pub extra: EventExtra,
}

/// Ce que le fichier contenait d'incompris.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub events: Vec<Event>,
    /// Événements ignorés (date illisible…).
    pub skipped: usize,
    /// Répétitions trop compliquées, réduites à leur première date.
    pub simplified: usize,
}

/// Déplie les lignes (une ligne qui commence par une espace continue la
/// précédente).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
        } else {
            lines.push(line.to_owned());
        }
    }
    lines
}

/// `NOM;PARAM=VALEUR:valeur` → (nom, paramètres, valeur).
fn split(line: &str) -> Option<(String, Params, &str)> {
    let mut in_quotes = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        if c == '"' {
            in_quotes = !in_quotes;
        }
        (c == ':' && !in_quotes).then_some(i)
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|part| part.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_owned(),
            )
        })
        .collect();
    Some((name, params, value))
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find_map(|(k, v)| (k == key).then_some(v.as_str()))
}

fn number<T: std::str::FromStr>(text: &str, range: std::ops::Range<usize>) -> Option<T> {
    text.get(range)?.parse().ok()
}

/// Une date ou une date et heure, en heure locale. Le booléen dit si c'est
/// une journée entière.
/// Une date lue dans le fichier, dans son fuseau.
struct Moment {
    civil: DateTime,
    all_day: bool,
    zone: Option<TimeZone>,
}

/// `civil` (dans `zone`) en heure locale.
fn localize(civil: DateTime, zone: Option<&TimeZone>, local: &TimeZone) -> Option<DateTime> {
    match zone {
        Some(zone) => {
            let instant: Timestamp = civil.to_zoned(zone.clone()).ok()?.timestamp();
            Some(instant.to_zoned(local.clone()).datetime())
        }
        // « Heure flottante » : déjà locale.
        None => Some(civil),
    }
}

/// Une date ou une date et heure, en heure locale. Le booléen dit si c'est
/// une journée entière.
fn moment(value: &str, params: &[(String, String)], local: &TimeZone) -> Option<(DateTime, bool)> {
    let read = read_moment(value, params)?;
    Some((
        localize(read.civil, read.zone.as_ref(), local)?,
        read.all_day,
    ))
}

fn read_moment(value: &str, params: &[(String, String)]) -> Option<Moment> {
    let value = value.trim();
    let date = Date::new(
        number(value, 0..4)?,
        number(value, 4..6)?,
        number(value, 6..8)?,
    )
    .ok()?;
    if value.len() == 8 || param(params, "VALUE") == Some("DATE") {
        return Some(Moment {
            civil: date.to_datetime(jiff::civil::Time::midnight()),
            all_day: true,
            zone: None,
        });
    }
    if value.as_bytes().get(8) != Some(&b'T') {
        return None;
    }
    let time = jiff::civil::Time::new(
        number(value, 9..11)?,
        number(value, 11..13)?,
        number(value, 13..15)?,
        0,
    )
    .ok()?;
    let zone = if value.ends_with('Z') {
        Some(TimeZone::UTC)
    } else {
        param(params, "TZID").and_then(|name| TimeZone::get(name).ok())
    };
    Some(Moment {
        civil: date.to_datetime(time),
        all_day: false,
        zone,
    })
}

/// `PT1H30M`, `P1D`, `P2W`.
fn duration(text: &str) -> Option<Span> {
    let text = text.trim();
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut rest = text.strip_prefix('P')?;
    let (mut weeks, mut days, mut hours, mut minutes, mut seconds) = (0i64, 0i64, 0i64, 0i64, 0i64);
    let mut in_time = false;
    let mut any = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('T') {
            in_time = true;
            rest = after;
            continue;
        }
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        let n: i64 = rest[..digits].parse().ok()?;
        let unit = rest[digits..].chars().next()?;
        let field = match (unit, in_time) {
            ('W', false) => &mut weeks,
            ('D', false) => &mut days,
            ('H', true) => &mut hours,
            ('M', true) => &mut minutes,
            ('S', true) => &mut seconds,
            _ => return None,
        };
        *field = field.checked_add(n)?;
        any = true;
        rest = &rest[digits + unit.len_utf8()..];
    }
    if !any {
        return None;
    }
    let span = Span::new()
        .try_weeks(weeks)
        .ok()?
        .try_days(days)
        .ok()?
        .try_hours(hours)
        .ok()?
        .try_minutes(minutes)
        .ok()?
        .try_seconds(seconds)
        .ok()?;
    Some(if negative { span.negate() } else { span })
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

/// `FREQ=WEEKLY;BYDAY=MO,TH`. `None` si la règle est trop compliquée.
fn rule(text: &str, local: &TimeZone) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
    };
    let mut freq = None;
    for part in text.split(';') {
        let (key, value) = part.split_once('=')?;
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                });
            }
            "INTERVAL" => rule.interval = value.parse().ok().filter(|n| *n >= 1)?,
            "COUNT" => rule.count = Some(value.parse().ok()?),
            "UNTIL" => rule.until = Some(moment(value, &[], local)?.0),
            "BYDAY" => {
                for day in value.split(',') {
                    let day = day.trim();
                    // Un fichier venu d'Internet : rien que de l'ASCII, sinon
                    // la règle est refusée (jamais de coupure au milieu d'un
                    // caractère).
                    if !day.is_ascii() {
                        return None;
                    }
                    let split = day.len().checked_sub(2)?;
                    let ordinal = match &day[..split] {
                        "" => None,
                        n => Some(n.trim_start_matches('+').parse::<i8>().ok()?),
                    };
                    rule.by_day.push((ordinal, weekday(&day[split..])?));
                }
            }
            "BYMONTHDAY" => {
                for day in value.split(',') {
                    rule.by_month_day.push(day.trim().parse().ok()?);
                }
            }
            "WKST" => {}
            _ => return None,
        }
    }
    rule.freq = freq?;
    Some(rule)
}

/// Lit le texte d'un fichier ICS.
#[must_use]
pub fn parse(text: &str, local: &TimeZone) -> Parsed {
    let mut parsed = Parsed::default();
    let mut current: Option<Vec<Property>> = None;
    let mut nested = 0usize;
    for line in unfold(text) {
        let Some((name, params, value)) = split(&line) else {
            continue;
        };
        match (name.as_str(), value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if current.is_none() => current = Some(Vec::new()),
            ("BEGIN", _) if current.is_some() => nested += 1,
            ("END", "VEVENT") if nested == 0 => {
                if let Some(properties) = current.take() {
                    match event(&properties, local, &mut parsed.simplified) {
                        Some(Read::Event(event)) if parsed.events.len() < MAX_EVENTS => {
                            parsed.events.push(*event);
                        }
                        Some(_) => {}
                        None => parsed.skipped += 1,
                    }
                }
            }
            ("END", _) if nested > 0 => nested -= 1,
            _ => {
                if nested == 0
                    && let Some(properties) = current.as_mut()
                {
                    properties.push((name, params, value.to_owned()));
                }
            }
        }
    }
    parsed
}

/// Un événement à partir de ses propriétés ; `None` s'il est illisible.
fn event(properties: &[Property], local: &TimeZone, simplified: &mut usize) -> Option<Read> {
    let get = |key: &str| properties.iter().find(|(name, _, _)| name == key);
    if get("STATUS").is_some_and(|(_, _, v)| v.trim().eq_ignore_ascii_case("CANCELLED")) {
        return Some(Read::Cancelled);
    }
    let (_, params, value) = get("DTSTART")?;
    let begin = read_moment(value, params)?;
    let all_day = begin.all_day;
    let zone = begin.zone;
    let source_start = begin.civil;
    // La fin, dans le fuseau du début.
    let source_end = if let Some((_, params, value)) = get("DTEND") {
        let end = read_moment(value, params)?;
        match (&end.zone, &zone) {
            (Some(end_zone), Some(zone)) => end
                .civil
                .to_zoned(end_zone.clone())
                .ok()?
                .with_time_zone(zone.clone())
                .datetime(),
            _ => end.civil,
        }
    } else if let Some((_, _, value)) = get("DURATION") {
        source_start.checked_add(duration(value)?).ok()?
    } else if all_day {
        source_start.checked_add(1.day()).ok()?
    } else {
        source_start
    };
    let source_end = source_end.max(source_start);
    let start = localize(source_start, zone.as_ref(), local)?;
    let end = localize(source_end, zone.as_ref(), local)?.max(start);
    let rule = match get("RRULE") {
        Some((_, _, text)) => {
            let parsed = rule(text, local);
            if parsed.is_none() {
                *simplified += 1;
            }
            parsed
        }
        None => None,
    };
    let exdates = properties
        .iter()
        .filter(|(name, _, _)| name == "EXDATE")
        .flat_map(|(_, params, value)| {
            value
                .split(',')
                .filter_map(|one| moment(one, params, local).map(|(at, _)| at))
                .collect::<Vec<_>>()
        })
        .collect();
    let recurrence_id = get("RECURRENCE-ID")
        .and_then(|(_, params, value)| moment(value, params, local).map(|(at, _)| at));
    let text = |key: &str| {
        get(key)
            .map(|(_, _, value)| unescape(value).trim().to_owned())
            .unwrap_or_default()
    };
    Some(Read::Event(Box::new(Event {
        uid: text("UID"),
        summary: {
            let summary = text("SUMMARY");
            if summary.is_empty() {
                "(sans titre)".into()
            } else {
                summary
            }
        },
        description: text("DESCRIPTION"),
        start,
        end,
        all_day,
        source_start,
        source_end,
        zone,
        rule,
        exdates,
        recurrence_id,
        extra: EventExtra {
            location: text("LOCATION").chars().take(200).collect(),
            // La première catégorie : l'étiquette (« Médecin »).
            label: get("CATEGORIES")
                .and_then(|(_, _, value)| {
                    value
                        .split(',')
                        .next()
                        .map(|first| unescape(first).trim().to_owned())
                })
                .unwrap_or_default()
                .chars()
                .take(60)
                .collect(),
            color: get("COLOR")
                .map(|(_, _, value)| value.trim().to_ascii_lowercase())
                .filter(|c| is_hex_color(c))
                .unwrap_or_default(),
            attendees: properties
                .iter()
                .filter(|(name, _, _)| name == "ATTENDEE")
                .filter_map(|(_, params, value)| {
                    let name = param(params, "CN").map_or_else(
                        || value.trim().trim_start_matches("mailto:").to_owned(),
                        |cn| cn.trim_matches('"').to_owned(),
                    );
                    let name: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
                    (!name.is_empty()).then_some(name)
                })
                .take(20)
                .collect(),
        },
    })))
}

/// `#rrggbb`.
#[must_use]
pub fn is_hex_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Le `n`-ième `weekday` du mois (`n` négatif : en partant de la fin).
fn nth_weekday(year: i16, month: i8, weekday: Weekday, n: i8) -> Option<Date> {
    Date::new(year, month, 1)
        .ok()?
        .nth_weekday_of_month(n, weekday)
        .ok()
}

/// Les dates candidates d'une période (une semaine, un mois…), dans l'ordre.
fn period_dates(rule: &Rule, start: Date, period: i64) -> Vec<Date> {
    let mut dates: Vec<Date> = match rule.freq {
        // BYDAY avec FREQ=DAILY : seulement ces jours-là (« en semaine »).
        Freq::Daily => start
            .checked_add((period * i64::from(rule.interval)).days())
            .ok()
            .into_iter()
            .filter(|day| {
                rule.by_day.is_empty() || rule.by_day.iter().any(|(_, w)| *w == day.weekday())
            })
            .collect(),
        Freq::Weekly => {
            let monday = start
                .checked_sub(i64::from(start.weekday().to_monday_zero_offset()).days())
                .ok();
            let Some(week) = monday.and_then(|m| {
                m.checked_add((period * i64::from(rule.interval)).weeks())
                    .ok()
            }) else {
                return Vec::new();
            };
            let days: Vec<Weekday> = if rule.by_day.is_empty() {
                vec![start.weekday()]
            } else {
                rule.by_day.iter().map(|(_, day)| *day).collect()
            };
            days.into_iter()
                .filter_map(|day| {
                    week.checked_add(i64::from(day.to_monday_zero_offset()).days())
                        .ok()
                })
                .collect()
        }
        Freq::Monthly => {
            let Some(first) = Date::new(start.year(), start.month(), 1)
                .ok()
                .and_then(|d| {
                    d.checked_add((period * i64::from(rule.interval)).months())
                        .ok()
                })
            else {
                return Vec::new();
            };
            let (year, month) = (first.year(), first.month());
            if !rule.by_day.is_empty() {
                rule.by_day
                    .iter()
                    .flat_map(|(ordinal, day)| match ordinal {
                        Some(n) => nth_weekday(year, month, *day, *n)
                            .into_iter()
                            .collect::<Vec<_>>(),
                        None => (1..=5)
                            .filter_map(|n| nth_weekday(year, month, *day, n))
                            .collect(),
                    })
                    .collect()
            } else if !rule.by_month_day.is_empty() {
                let last = first.last_of_month().day();
                rule.by_month_day
                    .iter()
                    .filter_map(|day| {
                        let day = if *day < 0 { last + day + 1 } else { *day };
                        Date::new(year, month, day).ok()
                    })
                    .collect()
            } else {
                Date::new(year, month, start.day())
                    .ok()
                    .into_iter()
                    .collect()
            }
        }
        Freq::Yearly => i16::try_from(i64::from(start.year()) + period * i64::from(rule.interval))
            .ok()
            .and_then(|year| Date::new(year, start.month(), start.day()).ok())
            .into_iter()
            .collect(),
    };
    dates.sort_unstable();
    dates.dedup();
    dates
}

/// La première période utile : sans `COUNT`, inutile de compter depuis un
/// début vieux de plusieurs années.
fn first_period(rule: &Rule, start: Date, from: Date) -> i64 {
    if rule.count.is_some() || from <= start {
        return 0;
    }
    let days = (from - start).get_days();
    let per = match rule.freq {
        Freq::Daily => 1,
        Freq::Weekly => 7,
        // Un peu moins qu'un mois ou qu'une année : on part avant, jamais après.
        Freq::Monthly => 31,
        Freq::Yearly => 366,
    } * i64::from(rule.interval);
    (i64::from(days) / per - 1).max(0)
}

/// Les occurrences de `events` qui touchent `[from, to]` (heure locale),
/// triées. Les répétitions se calculent dans le fuseau du fichier, puis
/// passent en heure locale (`local`).
#[must_use]
pub fn expand(events: &[Event], from: DateTime, to: DateTime, local: &TimeZone) -> Vec<Occurrence> {
    // Les occurrences remplacées par une version modifiée.
    let replaced: HashSet<(&str, DateTime)> = events
        .iter()
        .filter_map(|e| e.recurrence_id.map(|at| (e.uid.as_str(), at)))
        .collect();
    let mut out = Vec::new();
    for event in events {
        let length = event
            .source_start
            .until(event.source_end)
            .unwrap_or_default();
        // Une occurrence (dans le fuseau du fichier) : en heure locale.
        let mut push = |source_start: DateTime| {
            let Ok(source_end) = source_start.checked_add(length) else {
                return;
            };
            let (Some(start), Some(end)) = (
                localize(source_start, event.zone.as_ref(), local),
                localize(source_end, event.zone.as_ref(), local),
            ) else {
                return;
            };
            if end > from && start <= to {
                out.push(Occurrence {
                    uid: event.uid.clone(),
                    summary: event.summary.clone(),
                    description: event.description.clone(),
                    start,
                    end: end.max(start),
                    all_day: event.all_day,
                    extra: event.extra.clone(),
                });
            }
        };
        let Some(rule) = event
            .rule
            .as_ref()
            .filter(|_| event.recurrence_id.is_none())
        else {
            push(event.source_start);
            continue;
        };
        let time = event.source_start.time();
        let begin = first_period(rule, event.source_start.date(), from.date());
        let mut produced = 0usize;
        'periods: for period in begin..begin + MAX_STEPS {
            for date in period_dates(rule, event.source_start.date(), period) {
                let source = date.to_datetime(time);
                if source < event.source_start {
                    continue;
                }
                let Some(start) = localize(source, event.zone.as_ref(), local) else {
                    continue;
                };
                if rule.until.is_some_and(|until| start > until) || start > to {
                    break 'periods;
                }
                if rule.count.is_some_and(|count| produced >= count) {
                    break 'periods;
                }
                produced += 1;
                if event.exdates.contains(&start) || replaced.contains(&(event.uid.as_str(), start))
                {
                    continue;
                }
                push(source);
            }
        }
    }
    out.sort_by(|a, b| (a.start, &a.summary).cmp(&(b.start, &b.summary)));
    out.truncate(MAX_EVENTS);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use jiff::civil::date;

    fn paris() -> TimeZone {
        TimeZone::get("Europe/Paris").unwrap()
    }

    fn calendar(events: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//essai//FR\r\n{events}END:VCALENDAR\r\n"
        )
    }

    fn summaries(occurrences: &[Occurrence]) -> Vec<String> {
        occurrences
            .iter()
            .map(|o| format!("{} {}", o.start.strftime("%Y-%m-%d %H:%M"), o.summary))
            .collect()
    }

    #[test]
    fn simple_events_are_read() {
        let text = calendar(
            "BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Réunion\\, salle 2\r\nDESCRIPTION:Ligne 1\\nLigne 2\r\n\
             DTSTART:20261020T080000Z\r\nDTEND:20261020T090000Z\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Rappel\r\nEND:VALARM\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Vacances de la\r\n  Toussaint\r\n\
             DTSTART;VALUE=DATE:20261017\r\nDTEND;VALUE=DATE:20261102\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:c\r\nSUMMARY:Annulé\r\nSTATUS:CANCELLED\r\nDTSTART:20261021T080000\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:d\r\nSUMMARY:Dentiste\r\nDTSTART;TZID=America/New_York:20261021T090000\r\n\
             DURATION:PT30M\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:e\r\nSUMMARY:Cassé\r\nDTSTART:pas une date\r\nEND:VEVENT\r\n",
        );
        let parsed = parse(&text, &paris());
        assert_eq!(parsed.events.len(), 3);
        assert_eq!(parsed.skipped, 1);
        let meeting = &parsed.events[0];
        assert_eq!(meeting.summary, "Réunion, salle 2");
        assert_eq!(meeting.description, "Ligne 1\nLigne 2");
        // 8 h UTC = 10 h à Paris (heure d'été).
        assert_eq!(meeting.start, date(2026, 10, 20).at(10, 0, 0, 0));
        let holidays = &parsed.events[1];
        assert_eq!(holidays.summary, "Vacances de la Toussaint");
        assert!(holidays.all_day);
        assert_eq!(holidays.end, date(2026, 11, 2).at(0, 0, 0, 0));
        // 9 h à New York = 15 h à Paris ; 30 minutes.
        let dentist = &parsed.events[2];
        assert_eq!(dentist.start, date(2026, 10, 21).at(15, 0, 0, 0));
        assert_eq!(dentist.end, date(2026, 10, 21).at(15, 30, 0, 0));
    }

    #[test]
    fn weekly_waste_collection() {
        // Les poubelles le mardi, le recyclage un jeudi sur deux, sauf le
        // 29 octobre ; le 3 novembre déplacé au mercredi 4.
        let text = calendar(
            "BEGIN:VEVENT\r\nUID:om\r\nSUMMARY:Ordures ménagères\r\nDTSTART;VALUE=DATE:20261006\r\n\
             RRULE:FREQ=WEEKLY;BYDAY=TU\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:tri\r\nSUMMARY:Tri sélectif\r\nDTSTART;VALUE=DATE:20261015\r\n\
             RRULE:FREQ=WEEKLY;INTERVAL=2\r\nEXDATE;VALUE=DATE:20261029\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:om\r\nSUMMARY:Ordures ménagères (décalé)\r\nRECURRENCE-ID;VALUE=DATE:20261103\r\n\
             DTSTART;VALUE=DATE:20261104\r\nEND:VEVENT\r\n",
        );
        let parsed = parse(&text, &paris());
        let found = expand(
            &parsed.events,
            date(2026, 10, 16).at(0, 0, 0, 0),
            date(2026, 11, 13).at(0, 0, 0, 0),
            &paris(),
        );
        assert_eq!(
            summaries(&found),
            [
                "2026-10-20 00:00 Ordures ménagères",
                "2026-10-27 00:00 Ordures ménagères",
                "2026-11-04 00:00 Ordures ménagères (décalé)",
                "2026-11-10 00:00 Ordures ménagères",
                "2026-11-12 00:00 Tri sélectif",
            ]
        );
    }

    #[test]
    fn monthly_yearly_count_and_until() {
        let text = calendar(
            "BEGIN:VEVENT\r\nUID:conseil\r\nSUMMARY:Conseil\r\nDTSTART:20260908T200000\r\nDTEND:20260908T220000\r\n\
             RRULE:FREQ=MONTHLY;BYDAY=2TU;COUNT=4\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:loyer\r\nSUMMARY:Loyer\r\nDTSTART;VALUE=DATE:20260131\r\n\
             RRULE:FREQ=MONTHLY;BYMONTHDAY=-1;UNTIL=20261201\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:anniv\r\nSUMMARY:Anniversaire\r\nDTSTART;VALUE=DATE:20101103\r\n\
             RRULE:FREQ=YEARLY\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Compliqué\r\nDTSTART;VALUE=DATE:20261001\r\n\
             RRULE:FREQ=YEARLY;BYMONTH=10;BYSETPOS=1\r\nEND:VEVENT\r\n",
        );
        let parsed = parse(&text, &paris());
        assert_eq!(parsed.simplified, 1);
        let found = expand(
            &parsed.events,
            date(2026, 10, 1).at(0, 0, 0, 0),
            date(2027, 1, 31).at(0, 0, 0, 0),
            &paris(),
        );
        assert_eq!(
            summaries(&found),
            [
                "2026-10-01 00:00 Compliqué",
                "2026-10-13 20:00 Conseil",
                "2026-10-31 00:00 Loyer",
                "2026-11-03 00:00 Anniversaire",
                "2026-11-10 20:00 Conseil",
                "2026-11-30 00:00 Loyer",
                // COUNT=4 : septembre, octobre, novembre, décembre.
                "2026-12-08 20:00 Conseil",
            ]
        );
    }

    #[test]
    fn weekdays_only_with_a_daily_rule() {
        // Revue de la PR #60 : FREQ=DAILY;BYDAY=MO..FR donnait aussi le
        // samedi et le dimanche.
        let text = calendar(
            "BEGIN:VEVENT\r\nUID:car\r\nSUMMARY:Covoiturage\r\nDTSTART:20261019T073000\r\n\
             DURATION:PT30M\r\nRRULE:FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR\r\nEND:VEVENT\r\n",
        );
        let parsed = parse(&text, &paris());
        let found = expand(
            &parsed.events,
            date(2026, 10, 19).at(0, 0, 0, 0),
            date(2026, 10, 25).at(23, 0, 0, 0),
            &paris(),
        );
        let days: Vec<String> = found
            .iter()
            .map(|o| o.start.strftime("%a").to_string())
            .collect();
        assert_eq!(days, ["Mon", "Tue", "Wed", "Thu", "Fri"]);
    }

    #[test]
    fn repeats_keep_their_real_time_across_the_clock_change() {
        // Revue de la PR #60 : une répétition en heure universelle gardait
        // l'heure locale de la première date, une heure de travers l'été.
        let text = calendar(
            "BEGIN:VEVENT\r\nUID:utc\r\nSUMMARY:En UTC\r\nDTSTART:20260301T080000Z\r\n\
             DTEND:20260301T090000Z\r\nRRULE:FREQ=WEEKLY;COUNT=6\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:paris\r\nSUMMARY:À Paris\r\nDTSTART;TZID=Europe/Paris:20260301T090000\r\n\
             RRULE:FREQ=WEEKLY;COUNT=6\r\nEND:VEVENT\r\n",
        );
        let parsed = parse(&text, &paris());
        let found = expand(
            &parsed.events,
            date(2026, 3, 1).at(0, 0, 0, 0),
            date(2026, 4, 30).at(0, 0, 0, 0),
            &paris(),
        );
        let at = |summary: &str| -> Vec<String> {
            found
                .iter()
                .filter(|o| o.summary == summary)
                .map(|o| o.start.strftime("%d/%m %H:%M").to_string())
                .collect()
        };
        // 8 h UTC : 9 h l'hiver, 10 h l'été (passage le 29 mars).
        assert_eq!(
            at("En UTC"),
            [
                "01/03 09:00",
                "08/03 09:00",
                "15/03 09:00",
                "22/03 09:00",
                "29/03 10:00",
                "05/04 10:00"
            ]
        );
        // 9 h à Paris : toujours 9 h.
        assert_eq!(
            at("À Paris"),
            [
                "01/03 09:00",
                "08/03 09:00",
                "15/03 09:00",
                "22/03 09:00",
                "29/03 09:00",
                "05/04 09:00"
            ]
        );
        let first_summer = found
            .iter()
            .find(|o| o.summary == "En UTC" && o.start.month() == 4)
            .unwrap();
        assert_eq!(
            first_summer.end.strftime("%H:%M").to_string(),
            "11:00",
            "une heure de durée"
        );
    }

    #[test]
    fn strange_files_never_panic() {
        // Revue de la PR #60 : un caractère non ASCII dans BYDAY coupait une
        // chaîne au milieu d'un caractère.
        for rule in [
            "BYDAY=€",
            "BYDAY=1€",
            "BYDAY=é",
            "BYDAY=",
            "BYDAY=X",
            "BYMONTHDAY=€",
            "INTERVAL=€",
        ] {
            let text = calendar(&format!(
                "BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:x\r\nDTSTART;VALUE=DATE:20261006\r\n\
                 RRULE:FREQ=WEEKLY;{rule}\r\nEND:VEVENT\r\n"
            ));
            let parsed = parse(&text, &paris());
            assert_eq!(parsed.events.len(), 1, "{rule}");
            assert_eq!(
                parsed.simplified, 1,
                "{rule} : règle refusée, première date gardée"
            );
        }
        for broken in [
            "DTSTART:2026é1006",
            "DTSTART:€€€€€€€€T€€€€€€",
            "DTSTART;VALUE=DATE:2026100",
        ] {
            let text = calendar(&format!(
                "BEGIN:VEVENT\r\nUID:x\r\n{broken}\r\nEND:VEVENT\r\n"
            ));
            assert_eq!(parse(&text, &paris()).skipped, 1, "{broken}");
        }
        assert!(duration("Pé").is_none());
        assert!(duration("PT€H").is_none());
    }

    #[test]
    fn durations() {
        assert_eq!(
            duration("PT1H30M").unwrap().fieldwise(),
            1.hour().minutes(30).fieldwise()
        );
        assert_eq!(duration("P1D").unwrap().fieldwise(), 1.day().fieldwise());
        assert_eq!(duration("P2W").unwrap().fieldwise(), 2.weeks().fieldwise());
        assert!(duration("1H").is_none());
        assert!(duration("PT1X").is_none());
    }
}
