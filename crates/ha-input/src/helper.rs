//! Ce que fait chaque entrée quand on l'appelle (`toggle`, `set_value`,
//! `start`…), et l'état qu'elle affiche. Aucune attente ici : l'heure est
//! passée en paramètre, pour tester « le minuteur a démarré il y a 5 minutes ».

use std::time::Duration;

use ha_core::{Attributes, State};
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::config::{Helper, Kind, Number, parse_duration};

/// Où en est un minuteur.
#[derive(Debug, Clone, PartialEq)]
pub enum TimerState {
    Idle,
    /// En marche : il sonnera à `deadline`. `run` est la durée de ce tour
    /// (celle du fichier, ou celle donnée à `timer.start`).
    Active {
        deadline: Instant,
        finishes_at: Timestamp,
        run: Duration,
    },
    Paused {
        remaining: Duration,
        run: Duration,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Current {
    Boolean(bool),
    Number(f64),
    /// La position de l'option choisie.
    Select(usize),
    /// Date du dernier appui.
    Button(Option<Timestamp>),
    Timer(TimerState),
}

/// Une entrée et sa valeur actuelle.
#[derive(Debug, Clone)]
pub struct Entry {
    pub helper: Helper,
    value: Current,
}

/// Ce que l'appel a produit : un événement à publier (`timer.started`…).
pub type Fired = Option<&'static str>;

impl Entry {
    /// La valeur de départ : `initial` du fichier s'il y en a un, sinon la
    /// valeur d'avant l'arrêt du serveur (`restored`), sinon une valeur par
    /// défaut. Renvoie aussi `true` si un minuteur a sonné pendant l'arrêt.
    #[must_use]
    pub fn new(
        helper: Helper,
        restored: Option<&State>,
        now: Instant,
        wall: Timestamp,
    ) -> (Self, bool) {
        let restored_state = restored.map(State::state);
        let mut finished_while_down = false;
        let value = match &helper.kind {
            Kind::Boolean { initial } => {
                Current::Boolean(initial.unwrap_or(restored_state == Some("on")))
            }
            Kind::Number(number) => Current::Number(
                number
                    .initial
                    .or_else(|| restored_state.and_then(|s| s.parse().ok()))
                    .map_or(number.min, |value| number.normalize(value)),
            ),
            Kind::Select { options, initial } => {
                let wanted = initial.as_deref().or(restored_state);
                Current::Select(
                    wanted
                        .and_then(|wanted| options.iter().position(|o| o == wanted))
                        .unwrap_or(0),
                )
            }
            Kind::Button => Current::Button(restored_state.and_then(|s| s.parse().ok())),
            Kind::Timer { duration } => {
                let (state, finished) = restore_timer(restored, *duration, now, wall);
                finished_while_down = finished;
                Current::Timer(state)
            }
        };
        (Self { helper, value }, finished_while_down)
    }

    /// Un appel de service. Une erreur explique pourquoi l'appel est refusé.
    pub fn call(
        &mut self,
        service: &str,
        data: &Attributes,
        now: Instant,
        wall: Timestamp,
    ) -> Result<Fired, String> {
        let unknown = || Err(format!("service {service} inconnu"));
        match (&self.helper.kind, &mut self.value) {
            (Kind::Boolean { .. }, Current::Boolean(on)) => {
                *on = match service {
                    "turn_on" => true,
                    "turn_off" => false,
                    "toggle" => !*on,
                    _ => return unknown(),
                };
            }
            (Kind::Number(number), Current::Number(value)) => {
                *value = match service {
                    "set_value" => number.normalize(number_from(data.get("value"))?),
                    "increment" => number.normalize(*value + number.step),
                    "decrement" => number.normalize(*value - number.step),
                    _ => return unknown(),
                };
            }
            (Kind::Select { options, .. }, Current::Select(index)) => {
                let last = options.len() - 1;
                let cycle = data.get("cycle").and_then(Value::as_bool).unwrap_or(true);
                *index = match service {
                    "select_option" => {
                        let wanted = data.get("option").and_then(Value::as_str);
                        wanted
                            .and_then(|wanted| options.iter().position(|o| o == wanted))
                            .ok_or("option inconnue")?
                    }
                    "select_first" => 0,
                    "select_last" => last,
                    "select_next" if *index < last => *index + 1,
                    "select_next" => {
                        if cycle {
                            0
                        } else {
                            last
                        }
                    }
                    "select_previous" if *index > 0 => *index - 1,
                    "select_previous" => {
                        if cycle {
                            last
                        } else {
                            0
                        }
                    }
                    _ => return unknown(),
                };
            }
            (Kind::Button, Current::Button(last)) => {
                if service != "press" {
                    return unknown();
                }
                *last = Some(wall);
            }
            (Kind::Timer { duration }, Current::Timer(timer)) => {
                return timer_call(timer, *duration, service, data, now, wall);
            }
            _ => return Err("entrée incohérente".into()),
        }
        Ok(None)
    }

    /// Quand le minuteur doit sonner, s'il est en marche.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        match &self.value {
            Current::Timer(TimerState::Active { deadline, .. }) => Some(*deadline),
            _ => None,
        }
    }

    /// Le minuteur a-t-il sonné ? Il revient alors au repos.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.deadline().is_some_and(|deadline| deadline <= now) {
            self.value = Current::Timer(TimerState::Idle);
            return true;
        }
        false
    }

    /// L'état et les attributs à publier.
    #[must_use]
    pub fn render(&self, now: Instant) -> (String, Attributes) {
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), self.helper.name.clone().into());
        let state = match (&self.helper.kind, &self.value) {
            (Kind::Boolean { .. }, Current::Boolean(on)) => {
                if *on { "on" } else { "off" }.to_owned()
            }
            (Kind::Number(number), Current::Number(value)) => {
                attributes.insert("min".into(), json!(number.min));
                attributes.insert("max".into(), json!(number.max));
                attributes.insert("step".into(), json!(number.step));
                attributes.insert("mode".into(), number.mode.as_str().into());
                if let Some(unit) = &number.unit {
                    attributes.insert("unit_of_measurement".into(), unit.clone().into());
                }
                number.format(*value)
            }
            (Kind::Select { options, .. }, Current::Select(index)) => {
                attributes.insert("options".into(), json!(options));
                options.get(*index).cloned().unwrap_or_default()
            }
            (Kind::Button, Current::Button(last)) => {
                last.map_or_else(|| "unknown".to_owned(), |at| at.to_string())
            }
            (Kind::Timer { duration }, Current::Timer(timer)) => {
                let (state, run, remaining) = match timer {
                    TimerState::Idle => ("idle", *duration, *duration),
                    TimerState::Active {
                        deadline,
                        finishes_at,
                        run,
                    } => {
                        attributes.insert("finishes_at".into(), finishes_at.to_string().into());
                        ("active", *run, deadline.saturating_duration_since(now))
                    }
                    TimerState::Paused { remaining, run } => ("paused", *run, *remaining),
                };
                attributes.insert("duration".into(), hms(run).into());
                attributes.insert("remaining".into(), hms(remaining).into());
                state.to_owned()
            }
            _ => "unknown".to_owned(),
        };
        (state, attributes)
    }
}

impl Number {
    /// Ramène une valeur dans `[min, max]`, sur un pas de `step`.
    #[must_use]
    pub fn normalize(&self, value: f64) -> f64 {
        let steps = ((value - self.min) / self.step).round();
        let snapped = (self.min + steps * self.step).clamp(self.min, self.max);
        // Arrondi aux décimales du pas : 0.1 + 0.2 donne 0.3, pas 0.30000000000000004.
        let scale = 10_f64.powi(i32::from(self.decimals()));
        (snapped * scale).round() / scale
    }

    /// Le nombre de décimales du pas (`0.5` : 1, `5` : 0), 6 au plus.
    fn decimals(&self) -> u8 {
        (0..6)
            .find(|&decimals| {
                let scaled = self.step * 10_f64.powi(i32::from(decimals));
                (scaled - scaled.round()).abs() < 1e-9 * scaled.max(1.0)
            })
            .unwrap_or(6)
    }

    /// Le texte de l'état : `19.5`, `20` (avec les décimales du pas).
    #[must_use]
    pub fn format(&self, value: f64) -> String {
        format!("{value:.*}", usize::from(self.decimals()))
    }
}

/// Un nombre, écrit comme nombre (`21.5`) ou comme texte (`"21.5"`).
fn number_from(value: Option<&Value>) -> Result<f64, String> {
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    };
    number
        .filter(|number| number.is_finite())
        .ok_or_else(|| "« value » : un nombre est attendu".to_owned())
}

fn timer_call(
    timer: &mut TimerState,
    configured: Duration,
    service: &str,
    data: &Attributes,
    now: Instant,
    wall: Timestamp,
) -> Result<Fired, String> {
    let fired = match (service, &*timer) {
        ("start", current) => {
            let asked = data.get("duration").map(parse_duration).transpose()?;
            let (run, remaining, event) = match (current, asked) {
                // En pause, sans nouvelle durée : il repart où il en était.
                (TimerState::Paused { remaining, run }, None) => {
                    (*run, *remaining, "timer.started")
                }
                (TimerState::Active { run, .. }, asked) => {
                    let run = asked.unwrap_or(*run);
                    (run, run, "timer.restarted")
                }
                (_, asked) => {
                    let run = asked.unwrap_or(configured);
                    (run, run, "timer.started")
                }
            };
            if remaining.is_zero() {
                return Err("durée nulle : indique « duration »".into());
            }
            *timer = active(run, remaining, now, wall);
            Some(event)
        }
        ("pause", TimerState::Active { deadline, run, .. }) => {
            *timer = TimerState::Paused {
                remaining: deadline.saturating_duration_since(now),
                run: *run,
            };
            Some("timer.paused")
        }
        ("cancel", TimerState::Active { .. } | TimerState::Paused { .. }) => {
            *timer = TimerState::Idle;
            Some("timer.cancelled")
        }
        ("finish", TimerState::Active { .. } | TimerState::Paused { .. }) => {
            *timer = TimerState::Idle;
            Some("timer.finished")
        }
        // Mettre en pause un minuteur arrêté, etc. : rien à faire.
        ("pause" | "cancel" | "finish", _) => None,
        _ => return Err(format!("service {service} inconnu")),
    };
    Ok(fired)
}

fn active(run: Duration, remaining: Duration, now: Instant, wall: Timestamp) -> TimerState {
    let finishes_at = jiff::SignedDuration::try_from(remaining)
        .ok()
        .and_then(|remaining| wall.checked_add(remaining).ok())
        .unwrap_or(wall);
    TimerState::Active {
        deadline: now + remaining,
        finishes_at,
        run,
    }
}

/// Le minuteur d'avant l'arrêt du serveur. Renvoie `true` s'il a sonné
/// pendant l'arrêt.
fn restore_timer(
    restored: Option<&State>,
    configured: Duration,
    now: Instant,
    wall: Timestamp,
) -> (TimerState, bool) {
    let Some(restored) = restored else {
        return (TimerState::Idle, false);
    };
    let attribute = |name: &str| restored.attributes().get(name).and_then(Value::as_str);
    let run = attribute("duration")
        .and_then(parse_hms)
        .unwrap_or(configured);
    match restored.state() {
        "active" => {
            let Some(finishes_at) =
                attribute("finishes_at").and_then(|at| at.parse::<Timestamp>().ok())
            else {
                return (TimerState::Idle, false);
            };
            match wall
                .duration_until(finishes_at)
                .try_into()
                .ok()
                .filter(|remaining: &Duration| !remaining.is_zero())
            {
                Some(remaining) => (
                    TimerState::Active {
                        deadline: now + remaining,
                        finishes_at,
                        run,
                    },
                    false,
                ),
                None => (TimerState::Idle, true),
            }
        }
        "paused" => attribute("remaining")
            .and_then(parse_hms)
            .filter(|remaining| !remaining.is_zero())
            .map_or((TimerState::Idle, false), |remaining| {
                (TimerState::Paused { remaining, run }, false)
            }),
        _ => (TimerState::Idle, false),
    }
}

/// `1:05:09`, comme dans Home Assistant (secondes arrondies au-dessus : un
/// minuteur à 0,4 s affiche encore `0:00:01`).
#[must_use]
pub fn hms(duration: Duration) -> String {
    let seconds = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn parse_hms(text: &str) -> Option<Duration> {
    let mut parts = text.split(':');
    let (hours, minutes, seconds) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let hours: u64 = hours.parse().ok()?;
    let minutes: u64 = minutes.parse().ok()?;
    let seconds: u64 = seconds.parse().ok()?;
    (minutes < 60 && seconds < 60 && hours <= 24)
        .then(|| Duration::from_secs(hours * 3600 + minutes * 60 + seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    const YAML: &str = "input_boolean:\n  vacances:\n\
        input_number:\n  consigne: {min: 15, max: 23, step: 0.5, unit_of_measurement: °C}\n\
        input_select:\n  mode: {options: [Normal, Invités, Nuit]}\n\
        input_button:\n  sirene:\n\
        timer:\n  lessive: {duration: '00:10:00'}\n";

    fn entries(now: Instant) -> Vec<Entry> {
        parse(YAML)
            .unwrap()
            .helpers
            .into_iter()
            .map(|helper| Entry::new(helper, None, now, Timestamp::now()).0)
            .collect()
    }

    fn call(entry: &mut Entry, service: &str, data: Value, now: Instant) -> Result<Fired, String> {
        let Value::Object(data) = data else {
            panic!("un objet est attendu")
        };
        entry.call(service, &data, now, Timestamp::now())
    }

    fn state(entry: &Entry, now: Instant) -> String {
        entry.render(now).0
    }

    #[test]
    fn a_boolean_switches() {
        let now = Instant::now();
        let mut entry = entries(now).remove(0);
        assert_eq!(state(&entry, now), "off");
        call(&mut entry, "toggle", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "on");
        call(&mut entry, "turn_on", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "on");
        call(&mut entry, "turn_off", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "off");
        assert!(call(&mut entry, "open", json!({}), now).is_err());
    }

    #[test]
    fn a_number_stays_in_range_and_on_step() {
        let now = Instant::now();
        let mut entry = entries(now).remove(1);
        assert_eq!(state(&entry, now), "15.0");
        for (value, expected) in [
            (json!(19.3), "19.5"),
            (json!("21"), "21.0"),
            (json!(99), "23.0"),
            (json!(-4), "15.0"),
        ] {
            call(&mut entry, "set_value", json!({ "value": value }), now).unwrap();
            assert_eq!(state(&entry, now), expected, "{value}");
        }
        call(&mut entry, "increment", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "15.5");
        call(&mut entry, "decrement", json!({}), now).unwrap();
        call(&mut entry, "decrement", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "15.0");
        assert!(call(&mut entry, "set_value", json!({"value": "beaucoup"}), now).is_err());
        assert!(call(&mut entry, "set_value", json!({}), now).is_err());
        let (_, attributes) = entry.render(now);
        assert_eq!(attributes["unit_of_measurement"], "°C");
        assert_eq!(attributes["step"], 0.5);
    }

    #[test]
    fn a_select_moves_through_its_options() {
        let now = Instant::now();
        let mut entry = entries(now).remove(2);
        assert_eq!(state(&entry, now), "Normal");
        call(&mut entry, "select_option", json!({"option": "Nuit"}), now).unwrap();
        assert_eq!(state(&entry, now), "Nuit");
        call(&mut entry, "select_next", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "Normal", "on reprend au début");
        call(&mut entry, "select_previous", json!({"cycle": false}), now).unwrap();
        assert_eq!(state(&entry, now), "Normal", "sans boucler");
        call(&mut entry, "select_last", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "Nuit");
        call(&mut entry, "select_previous", json!({}), now).unwrap();
        assert_eq!(state(&entry, now), "Invités");
        assert!(call(&mut entry, "select_option", json!({"option": "Fête"}), now).is_err());
        assert_eq!(state(&entry, now), "Invités", "inchangé après un refus");
    }

    #[test]
    fn a_button_remembers_its_last_press() {
        let now = Instant::now();
        let mut entry = entries(now).remove(3);
        assert_eq!(state(&entry, now), "unknown");
        call(&mut entry, "press", json!({}), now).unwrap();
        assert!(state(&entry, now).parse::<Timestamp>().is_ok());
    }

    #[test]
    fn a_timer_runs_pauses_and_finishes() {
        let start = Instant::now();
        let mut entry = entries(start).remove(4);
        assert_eq!(state(&entry, start), "idle");
        assert_eq!(entry.render(start).1["remaining"], "0:10:00");
        assert_eq!(call(&mut entry, "pause", json!({}), start), Ok(None));

        assert_eq!(
            call(&mut entry, "start", json!({}), start),
            Ok(Some("timer.started"))
        );
        let later = start + Duration::from_mins(4);
        let (state_, attributes) = entry.render(later);
        assert_eq!(
            (state_.as_str(), &attributes["remaining"]),
            ("active", &json!("0:06:00"))
        );
        assert!(attributes["finishes_at"].as_str().is_some());

        assert_eq!(
            call(&mut entry, "pause", json!({}), later),
            Ok(Some("timer.paused"))
        );
        let much_later = later + Duration::from_hours(1);
        assert_eq!(
            entry.render(much_later).1["remaining"],
            "0:06:00",
            "figé en pause"
        );
        assert!(!entry.expire(much_later));

        assert_eq!(
            call(&mut entry, "start", json!({}), much_later),
            Ok(Some("timer.started"))
        );
        assert_eq!(entry.deadline(), Some(much_later + Duration::from_mins(6)));
        assert!(!entry.expire(much_later + Duration::from_secs(359)));
        assert!(entry.expire(much_later + Duration::from_mins(6)));
        assert_eq!(state(&entry, much_later), "idle");
    }

    #[test]
    fn a_timer_can_be_given_another_duration() {
        let now = Instant::now();
        let mut entry = entries(now).remove(4);
        call(&mut entry, "start", json!({"duration": "00:00:30"}), now).unwrap();
        assert_eq!(entry.render(now).1["duration"], "0:00:30");
        assert_eq!(
            call(&mut entry, "start", json!({"duration": 90}), now),
            Ok(Some("timer.restarted"))
        );
        assert_eq!(entry.deadline(), Some(now + Duration::from_secs(90)));
        assert_eq!(
            call(&mut entry, "cancel", json!({}), now),
            Ok(Some("timer.cancelled"))
        );
        assert_eq!(call(&mut entry, "cancel", json!({}), now), Ok(None));
        assert!(call(&mut entry, "start", json!({"duration": "toujours"}), now).is_err());
        assert!(call(&mut entry, "start", json!({"duration": 0}), now).is_err());
        assert_eq!(state(&entry, now), "idle");
    }

    #[test]
    fn values_come_back_after_a_restart() {
        let now = Instant::now();
        let wall = Timestamp::now();
        let helpers = parse(YAML).unwrap().helpers;
        let restored = |state: &str, attributes: Value| {
            let Value::Object(attributes) = attributes else {
                panic!("un objet est attendu")
            };
            let core = ha_core::Core::new();
            let id: ha_core::EntityId = "timer.x".parse().unwrap();
            core.states.set(id.clone(), state, attributes).unwrap();
            core.states.get(&id).unwrap()
        };
        let restore = |index: usize, state: &State| {
            Entry::new(helpers[index].clone(), Some(state), now, wall)
        };

        let (entry, _) = restore(0, &restored("on", json!({})));
        assert_eq!(entry.render(now).0, "on");
        let (entry, _) = restore(1, &restored("19.5", json!({})));
        assert_eq!(entry.render(now).0, "19.5");
        let (entry, _) = restore(2, &restored("Effacée", json!({})));
        assert_eq!(entry.render(now).0, "Normal", "option disparue du fichier");

        let in_5_min = wall + jiff::SignedDuration::from_mins(5);
        let (entry, finished) = restore(
            4,
            &restored(
                "active",
                json!({"finishes_at": in_5_min.to_string(), "duration": "0:10:00"}),
            ),
        );
        assert!(!finished);
        assert_eq!(entry.deadline(), Some(now + Duration::from_mins(5)));

        let (entry, finished) = restore(
            4,
            &restored(
                "active",
                json!({"finishes_at": (wall - jiff::SignedDuration::from_mins(1)).to_string()}),
            ),
        );
        assert!(finished, "il a sonné pendant l'arrêt");
        assert_eq!(entry.render(now).0, "idle");

        let (entry, _) = restore(4, &restored("paused", json!({"remaining": "0:03:00"})));
        assert_eq!(entry.render(now).0, "paused");
        assert_eq!(entry.render(now).1["remaining"], "0:03:00");
    }

    #[test]
    fn the_initial_value_wins_over_the_restored_one() {
        let helper = parse("input_boolean:\n  a: {initial: false}\n")
            .unwrap()
            .helpers
            .remove(0);
        let core = ha_core::Core::new();
        core.states
            .set(helper.entity_id.clone(), "on", Attributes::new())
            .unwrap();
        let restored = core.states.get(&helper.entity_id).unwrap();
        let (entry, _) = Entry::new(helper, Some(&restored), Instant::now(), Timestamp::now());
        assert_eq!(entry.render(Instant::now()).0, "off");
    }

    #[test]
    fn durations_are_shown_like_home_assistant() {
        assert_eq!(hms(Duration::from_secs(3909)), "1:05:09");
        assert_eq!(hms(Duration::from_millis(400)), "0:00:01");
        assert_eq!(parse_hms("1:05:09"), Some(Duration::from_secs(3909)));
        assert_eq!(parse_hms("0:61:00"), None);
    }
}
