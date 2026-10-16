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
    pub rule: Option<Rule>,
    pub exdates: Vec<DateTime>,
    /// L'occurrence d'un événement répété que celui-ci remplace.
    pub recurrence_id: Option<DateTime>,
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
fn moment(value: &str, params: &[(String, String)], local: &TimeZone) -> Option<(DateTime, bool)> {
    let value = value.trim();
    let date = Date::new(
        number(value, 0..4)?,
        number(value, 4..6)?,
        number(value, 6..8)?,
    )
    .ok()?;
    if value.len() == 8 || param(params, "VALUE") == Some("DATE") {
        return Some((date.to_datetime(jiff::civil::Time::midnight()), true));
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
    let civil = date.to_datetime(time);
    let zone = if value.ends_with('Z') {
        Some(TimeZone::UTC)
    } else {
        param(params, "TZID").and_then(|name| TimeZone::get(name).ok())
    };
    match zone {
        // Converti en heure locale.
        Some(zone) => {
            let instant: Timestamp = civil.to_zoned(zone).ok()?.timestamp();
            Some((instant.to_zoned(local.clone()).datetime(), false))
        }
        // « Heure flottante » : déjà locale.
        None => Some((civil, false)),
    }
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
    let (start, all_day) = moment(value, params, local)?;
    let end = if let Some((_, params, value)) = get("DTEND") {
        moment(value, params, local)?.0
    } else if let Some((_, _, value)) = get("DURATION") {
        start.checked_add(duration(value)?).ok()?
    } else if all_day {
        start.checked_add(1.day()).ok()?
    } else {
        start
    };
    let end = end.max(start);
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
        rule,
        exdates,
        recurrence_id,
    })))
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
        Freq::Daily => start
            .checked_add((period * i64::from(rule.interval)).days())
            .ok()
            .into_iter()
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

/// Les occurrences de `events` qui touchent `[from, to]`, triées.
#[must_use]
pub fn expand(events: &[Event], from: DateTime, to: DateTime) -> Vec<Occurrence> {
    // Les occurrences remplacées par une version modifiée.
    let replaced: HashSet<(&str, DateTime)> = events
        .iter()
        .filter_map(|e| e.recurrence_id.map(|at| (e.uid.as_str(), at)))
        .collect();
    let mut out = Vec::new();
    for event in events {
        let length = event.start.until(event.end).unwrap_or_default();
        let mut push = |start: DateTime| {
            let Ok(end) = start.checked_add(length) else {
                return;
            };
            if end > from && start <= to {
                out.push(Occurrence {
                    uid: event.uid.clone(),
                    summary: event.summary.clone(),
                    description: event.description.clone(),
                    start,
                    end,
                    all_day: event.all_day,
                });
            }
        };
        let Some(rule) = event
            .rule
            .as_ref()
            .filter(|_| event.recurrence_id.is_none())
        else {
            push(event.start);
            continue;
        };
        let time = event.start.time();
        let mut produced = 0usize;
        'periods: for period in 0..MAX_STEPS {
            let dates = period_dates(rule, event.start.date(), period);
            if dates.is_empty() && rule.freq != Freq::Monthly && rule.freq != Freq::Yearly {
                break;
            }
            for date in dates {
                let start = date.to_datetime(time);
                if start < event.start {
                    continue;
                }
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
                push(start);
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
