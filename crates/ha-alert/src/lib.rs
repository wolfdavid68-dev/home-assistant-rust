//! **Rappels insistants** (étape 69), comme l'intégration `alert` de Home
//! Assistant : tant qu'une chose reste dans un état à surveiller (la porte du
//! garage ouverte, le congélateur trop chaud), le serveur prévient, puis
//! redit, à intervalles choisis, jusqu'à ce que ce soit réglé ou que
//! quelqu'un touche « Compris ».
//!
//! ```yaml
//! # rappels.yaml
//! garage_ouvert:
//!   name: Garage ouvert
//!   entity_id: cover.garage
//!   state: open                 # l'état qui déclenche le rappel (« on » par défaut)
//!   repeat: [10, 30]            # minutes : 10, puis toutes les 30
//!   skip_first: true            # rien à l'ouverture : d'abord attendre 10 min
//!   message: >-
//!     Le garage est ouvert depuis
//!     {{ ((as_timestamp(now()) - as_timestamp(states.cover.garage.last_changed)) / 60) | round(0) }} min.
//!   done_message: Garage fermé.
//!   notifiers: [appli]          # notify.appli (par défaut), notify.telephone
//! ```
//!
//! Chaque rappel est une entité `alert.<nom>` :
//! - `idle` : rien à signaler ;
//! - `on` : en cours, les messages partent ;
//! - `off` : « Compris » a été touché, le serveur se tait jusqu'à ce que ce
//!   soit réglé.
//!
//! Sur le téléphone, chaque message remplace le précédent (même sujet) et
//! porte le bouton « Compris ». Le message de fin (« Garage fermé. ») ne part
//! que si au moins un rappel est parti.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ha_automation::Template;
use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Event, Origin, slugify};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{info, warn};

/// Le domaine des entités.
pub const DOMAIN: &str = "alert";
/// Le propriétaire des entités (journal, supervision).
const OWNER: &str = "alert";
/// Le bouton « Compris » d'un rappel : ce préfixe, puis son nom.
pub const BUTTON_PREFIX: &str = "RAPPEL_";
/// L'événement d'un bouton de notification touché (voir `ha-push`).
const ACTION_EVENT: &str = "notification_action";
/// Rappels au plus.
pub const MAX_ALERTS: usize = 50;
/// Intervalles au plus dans `repeat`.
const MAX_DELAYS: usize = 10;
/// Un nom de rappel tient dans un identifiant de bouton (40 caractères).
const MAX_ID_LEN: usize = 33;
/// Les messages : textes courts.
const MAX_TEXT: usize = 500;
/// Les seuls destinataires possibles.
const NOTIFIERS: [&str; 2] = ["notify.appli", "notify.telephone"];
/// Ce qu'on peut ajouter aux messages (`data:`).
const DATA_KEYS: [&str; 2] = ["users", "priority"];
/// Le rythme de la vérification.
const TICK: Duration = Duration::from_secs(1);

/// Un texte, fixe ou calculé (`{{ … }}`).
#[derive(Debug, Clone, PartialEq)]
pub enum Text {
    Plain(String),
    Template(Template),
}

impl Text {
    fn parse(text: &str, what: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() || text.chars().count() > MAX_TEXT {
            return Err(format!("{what} : de 1 à {MAX_TEXT} caractères"));
        }
        Ok(
            match Template::parse(text).map_err(|error| format!("{what} : {error}"))? {
                Some(template) => Self::Template(template),
                None => Self::Plain(text.to_owned()),
            },
        )
    }

    /// Le texte à envoyer, calculé maintenant.
    #[must_use]
    pub fn render(&self, states: &ha_core::StateMachine) -> String {
        let text = match self {
            Self::Plain(text) => return text.clone(),
            Self::Template(template) => match template.evaluate(states, None) {
                Value::String(text) => text,
                Value::Null => String::new(),
                other => other.to_string(),
            },
        };
        text.trim().chars().take(MAX_TEXT).collect()
    }

    fn entity_ids(&self) -> Vec<&EntityId> {
        match self {
            Self::Plain(_) => Vec::new(),
            Self::Template(template) => template.entity_ids().collect(),
        }
    }
}

/// Un rappel, vérifié.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    /// `alert.<id>`.
    pub entity_id: EntityId,
    pub id: String,
    pub name: String,
    /// L'entité surveillée.
    pub watched: EntityId,
    /// L'état qui déclenche le rappel.
    pub state: String,
    /// Les attentes entre deux messages ; la dernière se répète.
    pub repeat: Vec<Duration>,
    /// Attendre la première durée avant le premier message.
    pub skip_first: bool,
    /// Le bouton « Compris » (et `alert.turn_off`).
    pub can_acknowledge: bool,
    pub title: Option<Text>,
    /// `None` : le nom du rappel.
    pub message: Option<Text>,
    pub done_message: Option<Text>,
    pub notifiers: Vec<EntityId>,
    /// `users`, `priority`.
    pub data: Attributes,
}

impl Alert {
    /// L'attente numéro `index` (0 : la première) ; la dernière se répète.
    #[must_use]
    pub fn delay(&self, index: usize) -> Duration {
        self.repeat
            .get(index)
            .or_else(|| self.repeat.last())
            .copied()
            .unwrap_or(Duration::from_mins(60))
    }

    /// Le rythme, en français : « après 10 min, puis toutes les 30 min ».
    #[must_use]
    pub fn rhythm(&self) -> String {
        let minutes = |delay: &Duration| {
            let minutes = delay.as_secs_f64() / 60.0;
            format!("{} min", format!("{minutes}").replace('.', ","))
        };
        let (first, following) = if self.skip_first {
            let following = if self.repeat.len() > 1 {
                self.repeat.get(1..).unwrap_or_default()
            } else {
                &self.repeat[..]
            };
            (format!("après {}", minutes(&self.delay(0))), following)
        } else {
            ("tout de suite".to_owned(), &self.repeat[..])
        };
        let mut parts = vec![first];
        if let Some((last, middle)) = following.split_last() {
            parts.extend(
                middle
                    .iter()
                    .map(|delay| format!("{} plus tard", minutes(delay))),
            );
            parts.push(format!("toutes les {}", minutes(last)));
        }
        parts.join(", puis ")
    }

    /// L'identifiant du bouton « Compris ».
    #[must_use]
    pub fn button(&self) -> String {
        format!("{BUTTON_PREFIX}{}", self.id)
    }

    /// Les entités lues : la surveillée, et celles des messages.
    pub fn entity_ids(&self) -> impl Iterator<Item = &EntityId> {
        std::iter::once(&self.watched).chain(
            [&self.title, &self.message, &self.done_message]
                .into_iter()
                .flatten()
                .flat_map(Text::entity_ids),
        )
    }

    /// `Some(true)` : l'état à surveiller ; `None` : on ne sait pas
    /// (appareil absent ou injoignable) : rien ne change.
    #[must_use]
    pub fn firing(&self, states: &ha_core::StateMachine) -> Option<bool> {
        let state = states.get(&self.watched)?;
        match state.state() {
            "unavailable" | "unknown" => None,
            value => Some(value == self.state),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Repeat {
    One(f64),
    Many(Vec<f64>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlertConfig {
    name: String,
    entity_id: String,
    #[serde(default)]
    state: Option<String>,
    repeat: Repeat,
    #[serde(default)]
    skip_first: bool,
    #[serde(default = "yes")]
    can_acknowledge: bool,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    done_message: Option<String>,
    #[serde(default)]
    notifiers: Option<Vec<String>>,
    #[serde(default)]
    data: Option<serde_json::Map<String, Value>>,
}

fn yes() -> bool {
    true
}

/// Un texte court et propre (nom, état).
fn label(text: &str, what: &str, max: usize) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > max || text.chars().any(char::is_control) {
        return Err(format!(
            "{what} : de 1 à {max} caractères, sans retour à la ligne"
        ));
    }
    Ok(text.to_owned())
}

fn parse_one(id: &str, config: AlertConfig) -> Result<Alert, String> {
    if id.is_empty() || id.len() > MAX_ID_LEN || slugify(id) != id {
        return Err(format!(
            "« {id} » : un nom en minuscules, chiffres et « _ » (de 1 à {MAX_ID_LEN} caractères)"
        ));
    }
    let entity_id: EntityId = format!("{DOMAIN}.{id}")
        .parse()
        .map_err(|_| format!("« {id} » : nom impossible"))?;
    let name = label(&config.name, "name", 100)?;
    let watched: EntityId = config
        .entity_id
        .trim()
        .parse()
        .map_err(|_| format!("entity_id « {} » invalide", config.entity_id))?;
    if watched.domain() == DOMAIN {
        return Err(format!("{watched} : un rappel ne surveille pas un rappel"));
    }
    let state = match config.state {
        Some(state) => label(&state, "state", 100)?,
        None => "on".into(),
    };
    let minutes = match config.repeat {
        Repeat::One(minutes) => vec![minutes],
        Repeat::Many(list) => list,
    };
    if minutes.is_empty() || minutes.len() > MAX_DELAYS {
        return Err(format!("repeat : de 1 à {MAX_DELAYS} durées"));
    }
    let mut repeat = Vec::new();
    for minutes in minutes {
        if !(minutes.is_finite() && (1.0..=1440.0).contains(&minutes)) {
            return Err("repeat : des minutes, de 1 à 1440 (un jour)".into());
        }
        repeat.push(Duration::from_secs_f64(minutes * 60.0));
    }
    let text =
        |value: Option<String>, what: &str| value.map(|text| Text::parse(&text, what)).transpose();
    let notifiers = match config.notifiers {
        None => vec!["appli".to_owned()],
        Some(list) if list.is_empty() || list.len() > NOTIFIERS.len() => {
            return Err("notifiers : appli, telephone, ou les deux".into());
        }
        Some(list) => list,
    };
    let mut targets: Vec<EntityId> = Vec::new();
    for raw in &notifiers {
        let raw = raw.trim();
        let full = if raw.starts_with("notify.") {
            raw.to_owned()
        } else {
            format!("notify.{raw}")
        };
        if !NOTIFIERS.contains(&full.as_str()) {
            return Err(format!(
                "notifiers : « {raw} » inconnu (appli : les notifications du navigateur ; \
                 telephone : ntfy)"
            ));
        }
        let target: EntityId = full
            .parse()
            .map_err(|_| format!("notifiers : « {raw} » inconnu"))?;
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    let data = config.data.unwrap_or_default();
    if let Some(key) = data.keys().find(|key| !DATA_KEYS.contains(&key.as_str())) {
        return Err(format!(
            "data : « {key} » inconnu (seulement users et priority)"
        ));
    }
    Ok(Alert {
        entity_id,
        id: id.to_owned(),
        name,
        watched,
        state,
        repeat,
        skip_first: config.skip_first,
        can_acknowledge: config.can_acknowledge,
        title: text(config.title, "title")?,
        message: text(config.message, "message")?,
        done_message: text(config.done_message, "done_message")?,
        notifiers: targets,
        data,
    })
}

/// Lit et vérifie le contenu de `rappels.yaml`.
///
/// # Errors
/// Le fichier est mal écrit, ou un rappel est faux (le message dit lequel).
pub fn parse_alerts(text: &str) -> Result<Vec<Alert>, String> {
    let file: BTreeMap<String, AlertConfig> =
        serde_saphyr::from_str(text).map_err(|error| error.to_string())?;
    if file.len() > MAX_ALERTS {
        return Err(format!("{MAX_ALERTS} rappels au plus"));
    }
    file.into_iter()
        .map(|(id, config)| parse_one(&id, config).map_err(|error| format!("{id} : {error}")))
        .collect()
}

/// Lit `rappels.yaml` ; une liste vide si le fichier est absent.
///
/// # Errors
/// Le fichier existe mais ne peut pas être lu, ou il est faux.
pub fn load(path: &Path) -> Result<Vec<Alert>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_alerts(&text).map_err(|error| format!("{} : {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!(
            "lecture de {} impossible : {error}",
            path.display()
        )),
    }
}

/// Où en est un rappel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Rien à signaler.
    Idle,
    /// En cours : les messages partent.
    Active,
    /// « Compris » : plus de message jusqu'à ce que ce soit réglé.
    Acknowledged,
}

impl Phase {
    /// L'état de l'entité, comme dans Home Assistant.
    #[must_use]
    pub fn state(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Active => "on",
            Self::Acknowledged => "off",
        }
    }
}

/// Ce qu'il faut envoyer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Send {
    /// Le message (premier envoi ou rappel).
    Reminder,
    /// Le message de fin : c'est réglé.
    Done,
}

/// Le suivi d'un rappel, sans horloge ni réseau : on lui donne l'heure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracker {
    phase: Phase,
    /// Messages envoyés depuis le début.
    sent: u32,
    /// Attentes déjà prévues (pour choisir la suivante dans `repeat`).
    waits: usize,
    next: Option<Instant>,
    /// Le même moment, en heure réelle, pour l'afficher.
    next_at: Option<jiff::Timestamp>,
    /// Qui a touché « Compris ».
    by: Option<String>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            sent: 0,
            waits: 0,
            next: None,
            next_at: None,
            by: None,
        }
    }
}

impl Tracker {
    /// Reprend après un redémarrage : un rappel en cours (ou « compris »)
    /// le reste, sans redire tout de suite ce qui vient d'être dit.
    #[must_use]
    pub fn resume(alert: &Alert, state: &str, sent: u32, by: Option<String>, now: Instant) -> Self {
        let phase = match state {
            "on" => Phase::Active,
            "off" => Phase::Acknowledged,
            _ => return Self::default(),
        };
        let mut tracker = Self {
            phase,
            sent,
            waits: usize::try_from(sent).unwrap_or(usize::MAX),
            next: None,
            next_at: None,
            by: (phase == Phase::Acknowledged).then_some(by).flatten(),
        };
        tracker.schedule(alert, now);
        tracker
    }

    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    #[must_use]
    pub fn sent(&self) -> u32 {
        self.sent
    }

    /// Qui a touché « Compris ».
    #[must_use]
    pub fn acknowledged_by(&self) -> Option<&str> {
        self.by.as_deref()
    }

    /// Le prochain message, s'il y en a un de prévu.
    #[must_use]
    pub fn next(&self) -> Option<Instant> {
        match self.phase {
            Phase::Active => self.next,
            _ => None,
        }
    }

    /// L'heure réelle du prochain message, s'il y en a un de prévu.
    #[must_use]
    pub fn next_at(&self) -> Option<jiff::Timestamp> {
        self.next().and(self.next_at)
    }

    fn schedule(&mut self, alert: &Alert, now: Instant) {
        let delay = alert.delay(self.waits);
        self.next = Some(now + delay);
        self.next_at = jiff::SignedDuration::try_from(delay)
            .ok()
            .and_then(|delay| jiff::Timestamp::now().checked_add(delay).ok())
            .and_then(|at| at.round(jiff::Unit::Second).ok());
        self.waits = self.waits.saturating_add(1);
    }

    /// Avance d'après l'état surveillé (`firing`) et l'heure. Dit ce qu'il
    /// faut envoyer.
    pub fn update(&mut self, alert: &Alert, firing: Option<bool>, now: Instant) -> Option<Send> {
        let firing = firing?;
        match (self.phase, firing) {
            (Phase::Idle, false) => None,
            (Phase::Active | Phase::Acknowledged, false) => {
                let done = self.sent > 0 && alert.done_message.is_some();
                *self = Self::default();
                done.then_some(Send::Done)
            }
            (Phase::Idle, true) => {
                *self = Self {
                    phase: Phase::Active,
                    ..Self::default()
                };
                if alert.skip_first {
                    self.schedule(alert, now);
                } else {
                    self.next = Some(now);
                    self.next_at = None;
                }
                self.due(alert, now)
            }
            (Phase::Active, true) => self.due(alert, now),
            (Phase::Acknowledged, true) => {
                // Le rythme continue en silence : « annuler Compris » reprend
                // au rappel suivant, pas tout de suite.
                if self.next.is_some_and(|next| next <= now) {
                    self.schedule(alert, now);
                }
                None
            }
        }
    }

    fn due(&mut self, alert: &Alert, now: Instant) -> Option<Send> {
        if self.next.is_some_and(|next| next <= now) {
            self.sent = self.sent.saturating_add(1);
            self.schedule(alert, now);
            return Some(Send::Reminder);
        }
        None
    }

    /// « Compris » (par `who`) : `true` si le rappel était en cours.
    pub fn acknowledge(&mut self, who: &str) -> bool {
        if self.phase == Phase::Active {
            self.phase = Phase::Acknowledged;
            self.by = Some(who.to_owned());
            return true;
        }
        false
    }

    /// Annule « Compris » : les rappels reprennent au suivant prévu.
    pub fn unacknowledge(&mut self) -> bool {
        if self.phase == Phase::Acknowledged {
            self.phase = Phase::Active;
            self.by = None;
            return true;
        }
        false
    }
}

/// Les données de `notify.send_message` pour un destinataire.
#[must_use]
pub fn message_data(
    alert: &Alert,
    send: Send,
    notifier: &EntityId,
    states: &ha_core::StateMachine,
) -> Option<Attributes> {
    let message = match send {
        Send::Reminder => alert
            .message
            .as_ref()
            .map_or_else(|| alert.name.clone(), |text| text.render(states)),
        Send::Done => alert.done_message.as_ref()?.render(states),
    };
    if message.is_empty() {
        return None;
    }
    let title = alert
        .title
        .as_ref()
        .map(|text| text.render(states))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| alert.name.clone());
    let mut data = alert.data.clone();
    data.insert("message".into(), message.into());
    data.insert("title".into(), title.into());
    if notifier.as_str() == "notify.appli" {
        // Le même sujet : chaque message remplace le précédent sur le
        // téléphone, le message de fin aussi.
        data.insert("tag".into(), format!("rappel_{}", alert.id).into());
        if send == Send::Reminder && alert.can_acknowledge {
            data.insert(
                "actions".into(),
                json!([{"action": alert.button(), "title": "Compris"}]),
            );
        }
    } else {
        data.remove("users");
    }
    Some(data)
}

/// Les rappels, suivis en arrière-plan.
#[derive(Debug)]
pub struct Alerts(Background);

impl Alerts {
    /// Publie chaque rappel, puis le suit. L'abonnement au bus est fait avant
    /// de rendre la main.
    #[must_use]
    pub fn start(alerts: Vec<Alert>, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("les rappels", move |token| {
            run(alerts, core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

fn publish(core: &Core, alert: &Alert, tracker: &Tracker) {
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), alert.name.clone().into());
    attributes.insert("icon".into(), "mdi:bell-ring".into());
    attributes.insert("surveille".into(), alert.watched.as_str().into());
    attributes.insert("etat_surveille".into(), alert.state.clone().into());
    attributes.insert("rappels".into(), tracker.sent().into());
    attributes.insert("acquittable".into(), alert.can_acknowledge.into());
    if let Some(who) = tracker.acknowledged_by() {
        attributes.insert("compris_par".into(), who.into());
    }
    if let Some(at) = tracker.next_at() {
        attributes.insert("prochain_rappel".into(), at.to_string().into());
    }
    let value = tracker.phase().state();
    // Ne republie que ce qui change.
    let same = core
        .states
        .get(&alert.entity_id)
        .is_some_and(|state| state.state() == value && state.attributes() == &attributes);
    if same {
        return;
    }
    if let Err(error) = core
        .states
        .set_by(OWNER, alert.entity_id.clone(), value, attributes)
    {
        warn!("{} : état refusé ({error})", alert.entity_id);
    }
}

fn send(core: &Core, alert: &Alert, send: Send) {
    info!(
        "rappel {} : {}",
        alert.entity_id,
        match send {
            Send::Reminder => "message envoyé",
            Send::Done => "c'est réglé",
        }
    );
    for notifier in &alert.notifiers {
        let Some(data) = message_data(alert, send, notifier, &core.states) else {
            continue;
        };
        if let Err(error) = core.call_service_from(
            Origin::Automation(alert.name.clone()),
            "notify",
            "send_message",
            notifier.clone(),
            data,
        ) {
            warn!("rappel {} : {notifier} refusé ({error})", alert.entity_id);
        }
    }
}

/// Avance un rappel, envoie ce qu'il faut, et publie son état.
fn step(core: &Core, alert: &Alert, tracker: &mut Tracker, now: Instant) {
    let firing = alert.firing(&core.states);
    if let Some(what) = tracker.update(alert, firing, now) {
        send(core, alert, what);
    }
    publish(core, alert, tracker);
}

async fn run(
    alerts: Vec<Alert>,
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    let now = Instant::now();
    // Reprise après un redémarrage : l'état restauré dit où on en était.
    let mut trackers: Vec<Tracker> = alerts
        .iter()
        .map(|alert| {
            let previous = core.states.get(&alert.entity_id);
            let state = previous.as_ref().map_or("idle", |state| state.state());
            let sent = previous
                .as_ref()
                .and_then(|state| state.attributes().get("rappels")?.as_u64())
                .and_then(|sent| u32::try_from(sent).ok())
                .unwrap_or(0);
            let by = previous
                .as_ref()
                .and_then(|state| state.attributes().get("compris_par")?.as_str())
                .map(str::to_owned);
            Tracker::resume(alert, state, sent, by, now)
        })
        .collect();
    let mut readers: HashMap<EntityId, Vec<usize>> = HashMap::new();
    for (index, alert) in alerts.iter().enumerate() {
        readers
            .entry(alert.watched.clone())
            .or_default()
            .push(index);
    }
    let by_button: HashMap<String, usize> = alerts
        .iter()
        .enumerate()
        .map(|(index, alert)| (alert.button(), index))
        .collect();
    info!("rappels : {}", alerts.len());
    for (alert, tracker) in alerts.iter().zip(trackers.iter_mut()) {
        step(&core, alert, tracker, now);
    }
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            _ = ticker.tick() => {
                let now = Instant::now();
                for (alert, tracker) in alerts.iter().zip(trackers.iter_mut()) {
                    if tracker.next().is_some_and(|next| next <= now)
                        || tracker.phase() == Phase::Acknowledged
                    {
                        step(&core, alert, tracker, now);
                    }
                }
            }
            received = events.recv() => match received {
                Ok(event) => {
                    let now = Instant::now();
                    for index in affected(&event, &readers, &by_button, &alerts) {
                        let (Some(alert), Some(tracker)) = (alerts.get(index), trackers.get_mut(index)) else {
                            continue;
                        };
                        handle(&event, alert, tracker);
                        step(&core, alert, tracker, now);
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    warn!("rappels : {missed} événements manqués, tout est revérifié");
                    let now = Instant::now();
                    for (alert, tracker) in alerts.iter().zip(trackers.iter_mut()) {
                        step(&core, alert, tracker, now);
                    }
                }
                Err(RecvError::Closed) => break,
            },
        }
    }
}

/// Les rappels concernés par un événement.
fn affected(
    event: &Event,
    readers: &HashMap<EntityId, Vec<usize>>,
    by_button: &HashMap<String, usize>,
    alerts: &[Alert],
) -> Vec<usize> {
    match event {
        Event::StateChanged { entity_id, .. } => {
            readers.get(entity_id).cloned().unwrap_or_default()
        }
        Event::CallService {
            domain, entity_id, ..
        } if domain == DOMAIN => alerts
            .iter()
            .position(|alert| &alert.entity_id == entity_id)
            .into_iter()
            .collect(),
        Event::Fired {
            event_type, data, ..
        } if event_type == ACTION_EVENT => data
            .get("action")
            .and_then(Value::as_str)
            .and_then(|action| by_button.get(action))
            .copied()
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// Un ordre (`alert.turn_off`…) ou le bouton « Compris ».
fn handle(event: &Event, alert: &Alert, tracker: &mut Tracker) {
    let (service, who) = match event {
        Event::CallService {
            service, origin, ..
        } => (
            service.as_str(),
            match origin {
                Origin::User(name) | Origin::Agent(name) => name.clone(),
                other => other.describe().unwrap_or_else(|| "le serveur".into()),
            },
        ),
        Event::Fired { data, .. } => (
            "turn_off",
            data.get("user")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned(),
        ),
        Event::StateChanged { .. } => return,
    };
    let wanted_ack = match service {
        "turn_off" => true,
        "turn_on" => false,
        "toggle" => tracker.phase() == Phase::Active,
        _ => return,
    };
    if wanted_ack {
        if !alert.can_acknowledge {
            warn!(
                "rappel {} : « Compris » refusé (can_acknowledge: false)",
                alert.entity_id
            );
            return;
        }
        if tracker.acknowledge(&who) {
            info!("rappel {} : compris ({who})", alert.entity_id);
        }
    } else if tracker.unacknowledge() {
        info!("rappel {} : reprend ({who})", alert.entity_id);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

    use super::*;

    const FILE: &str = r#"
garage_ouvert:
  name: Garage ouvert
  entity_id: cover.garage
  state: open
  repeat: [10, 30]
  skip_first: true
  message: "Le garage est {{ states('cover.garage') }}."
  done_message: Garage fermé.
  notifiers: [appli, notify.telephone]
  data: {users: [david], priority: 4}
congelateur:
  name: Congélateur trop chaud
  entity_id: binary_sensor.congelateur_chaud
  repeat: 5
  can_acknowledge: false
"#;

    fn alerts() -> Vec<Alert> {
        parse_alerts(FILE).unwrap()
    }

    fn garage() -> Alert {
        alerts().remove(1)
    }

    fn mins(n: u64) -> Duration {
        Duration::from_mins(n)
    }

    #[test]
    fn the_file_is_checked() {
        let alerts = alerts();
        let ids: Vec<&str> = alerts.iter().map(|a| a.entity_id.as_str()).collect();
        assert_eq!(ids, ["alert.congelateur", "alert.garage_ouvert"]);
        let garage = &alerts[1];
        assert_eq!(garage.repeat, [mins(10), mins(30)]);
        assert_eq!(garage.delay(0), mins(10));
        assert_eq!(garage.delay(5), mins(30), "la dernière se répète");
        assert_eq!(garage.notifiers.len(), 2);
        assert_eq!(garage.button(), "RAPPEL_garage_ouvert");
        assert_eq!(garage.rhythm(), "après 10 min, puis toutes les 30 min");
        assert_eq!(alerts[0].rhythm(), "tout de suite, puis toutes les 5 min");
        let reads: Vec<&str> = garage.entity_ids().map(EntityId::as_str).collect();
        assert_eq!(reads, ["cover.garage", "cover.garage"]);
        let freezer = &alerts[0];
        assert_eq!(freezer.state, "on");
        assert!(!freezer.can_acknowledge && !freezer.skip_first);
        assert_eq!(freezer.notifiers[0].as_str(), "notify.appli");
        let base = "  name: A\n  entity_id: binary_sensor.a\n";
        for (bad, hint) in [
            (format!("A:\n{base}  repeat: 5\n"), "minuscules"),
            (format!("a-b:\n{base}  repeat: 5\n"), "minuscules"),
            (
                format!("{}:\n{base}  repeat: 5\n", "a".repeat(34)),
                "minuscules",
            ),
            (format!("a:\n{base}  repeat: 0\n"), "repeat"),
            (format!("a:\n{base}  repeat: 2000\n"), "repeat"),
            (format!("a:\n{base}  repeat: []\n"), "repeat"),
            (
                format!("a:\n{base}  repeat: 5\n  notifiers: [alexa]\n"),
                "alexa",
            ),
            (
                format!("a:\n{base}  repeat: 5\n  data: {{url: x}}\n"),
                "url",
            ),
            (
                format!("a:\n{base}  repeat: 5\n  couleur: bleu\n"),
                "couleur",
            ),
            (
                format!("a:\n{base}  repeat: 5\n  message: \"{{{{ 1 +\"\n"),
                "message",
            ),
            (
                "a:\n  name: A\n  entity_id: alert.b\n  repeat: 5\n".into(),
                "rappel",
            ),
            (
                "a:\n  name: A\n  entity_id: pas_un_id\n  repeat: 5\n".into(),
                "entity_id",
            ),
        ] {
            let error = parse_alerts(&bad).unwrap_err();
            assert!(error.contains(hint), "{bad} : {error}");
        }
        let example = parse_alerts(include_str!("../../../rappels.example.yaml")).unwrap();
        assert_eq!(example.len(), 3);
    }

    #[test]
    fn reminders_follow_the_rhythm_until_it_is_settled() {
        let alert = garage();
        let start = Instant::now();
        let at = |minutes: u64| start + mins(minutes);
        let mut tracker = Tracker::default();
        assert_eq!(tracker.update(&alert, Some(false), at(0)), None);
        // Ouvert : skip_first, rien tout de suite.
        assert_eq!(tracker.update(&alert, Some(true), at(0)), None);
        assert_eq!(tracker.phase(), Phase::Active);
        assert_eq!(tracker.next(), Some(at(10)));
        assert_eq!(tracker.update(&alert, Some(true), at(9)), None);
        assert_eq!(
            tracker.update(&alert, Some(true), at(10)),
            Some(Send::Reminder)
        );
        assert_eq!(tracker.next(), Some(at(40)), "puis toutes les 30 min");
        assert_eq!(tracker.update(&alert, Some(true), at(39)), None);
        assert_eq!(
            tracker.update(&alert, Some(true), at(40)),
            Some(Send::Reminder)
        );
        assert_eq!(tracker.next(), Some(at(70)));
        assert_eq!(tracker.sent(), 2);
        // Injoignable : on ne sait pas, rien ne change, rien ne part.
        assert_eq!(tracker.update(&alert, None, at(80)), None);
        assert_eq!(tracker.phase(), Phase::Active);
        // Fermé : le message de fin.
        assert_eq!(
            tracker.update(&alert, Some(false), at(81)),
            Some(Send::Done)
        );
        assert_eq!(tracker, Tracker::default());
        // Ouvert puis refermé avant le premier message : pas de « fermé ».
        tracker.update(&alert, Some(true), at(100));
        assert_eq!(tracker.update(&alert, Some(false), at(105)), None);
    }

    #[test]
    fn without_skip_first_the_first_message_leaves_at_once() {
        let alert = alerts().remove(0);
        let start = Instant::now();
        let mut tracker = Tracker::default();
        assert_eq!(
            tracker.update(&alert, Some(true), start),
            Some(Send::Reminder)
        );
        assert_eq!(tracker.next(), Some(start + mins(5)));
        // Pas de done_message : rien à la fin.
        assert_eq!(tracker.update(&alert, Some(false), start + mins(1)), None);
    }

    #[test]
    fn understood_silences_until_it_is_settled() {
        let alert = garage();
        let start = Instant::now();
        let at = |minutes: u64| start + mins(minutes);
        let mut tracker = Tracker::default();
        tracker.update(&alert, Some(true), at(0));
        tracker.update(&alert, Some(true), at(10));
        assert!(tracker.acknowledge("david"));
        assert!(!tracker.acknowledge("alice"), "déjà compris");
        assert_eq!(tracker.phase(), Phase::Acknowledged);
        assert_eq!(tracker.acknowledged_by(), Some("david"));
        assert_eq!(tracker.next(), None);
        assert_eq!(tracker.update(&alert, Some(true), at(40)), None);
        assert_eq!(tracker.update(&alert, Some(true), at(70)), None);
        // Annuler « Compris » : reprise au rappel suivant prévu (100), pas
        // tout de suite.
        assert!(tracker.unacknowledge());
        assert_eq!(tracker.next(), Some(at(100)));
        assert_eq!(tracker.update(&alert, Some(true), at(75)), None);
        assert_eq!(
            tracker.update(&alert, Some(true), at(100)),
            Some(Send::Reminder)
        );
        // Compris puis fermé : le message de fin part quand même.
        tracker.acknowledge("david");
        assert_eq!(
            tracker.update(&alert, Some(false), at(101)),
            Some(Send::Done)
        );
    }

    #[test]
    fn a_restart_does_not_repeat_at_once() {
        let alert = garage();
        let now = Instant::now();
        let tracker = Tracker::resume(&alert, "on", 2, None, now);
        assert_eq!(tracker.phase(), Phase::Active);
        assert_eq!(tracker.next(), Some(now + mins(30)));
        // L'heure affichée est celle du nouveau calcul, pas l'ancienne.
        let shown = tracker.next_at().unwrap();
        let wait = shown.duration_since(jiff::Timestamp::now()).as_secs();
        assert!((1790..=1800).contains(&wait), "{wait}");
        let tracker = Tracker::resume(&alert, "off", 1, Some("alice".into()), now);
        assert_eq!(tracker.acknowledged_by(), Some("alice"));
        assert_eq!(
            Tracker::resume(&alert, "idle", 0, None, now),
            Tracker::default()
        );
    }

    #[test]
    fn messages_carry_the_button_and_replace_each_other() {
        let alert = garage();
        let core = Core::new();
        core.states
            .set("cover.garage".parse().unwrap(), "open", Attributes::new())
            .unwrap();
        let appli: EntityId = "notify.appli".parse().unwrap();
        let phone: EntityId = "notify.telephone".parse().unwrap();
        let data = message_data(&alert, Send::Reminder, &appli, &core.states).unwrap();
        assert_eq!(data["message"], "Le garage est open.");
        assert_eq!(data["title"], "Garage ouvert");
        assert_eq!(data["tag"], "rappel_garage_ouvert");
        assert_eq!(data["users"], json!(["david"]));
        assert_eq!(data["priority"], 4);
        assert_eq!(
            data["actions"],
            json!([{"action": "RAPPEL_garage_ouvert", "title": "Compris"}])
        );
        let done = message_data(&alert, Send::Done, &appli, &core.states).unwrap();
        assert_eq!(done["message"], "Garage fermé.");
        assert_eq!(done["tag"], "rappel_garage_ouvert");
        assert!(done.get("actions").is_none());
        let sms = message_data(&alert, Send::Reminder, &phone, &core.states).unwrap();
        assert!(sms.get("tag").is_none() && sms.get("users").is_none());
        // Sans message : le nom ; sans « Compris » : sans bouton.
        let freezer = alerts().remove(0);
        let data = message_data(&freezer, Send::Reminder, &appli, &core.states).unwrap();
        assert_eq!(data["message"], "Congélateur trop chaud");
        assert!(data.get("actions").is_none());
        assert!(message_data(&freezer, Send::Done, &appli, &core.states).is_none());
    }

    fn sent_messages(rx: &mut tokio::sync::broadcast::Receiver<Arc<Event>>) -> Vec<String> {
        let mut messages = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let Event::CallService {
                domain,
                entity_id,
                data,
                ..
            } = event.as_ref()
                && domain == "notify"
                && entity_id.as_str() == "notify.appli"
            {
                messages.push(data["message"].as_str().unwrap_or_default().to_owned());
            }
        }
        messages
    }

    #[tokio::test(start_paused = true)]
    async fn the_service_reminds_and_listens_to_understood() {
        let core = Arc::new(Core::new());
        let garage: EntityId = "cover.garage".parse().unwrap();
        let entity: EntityId = "alert.garage_ouvert".parse().unwrap();
        core.states
            .set(garage.clone(), "closed", Attributes::new())
            .unwrap();
        let mut rx = core.bus.subscribe();
        let service = Alerts::start(alerts(), Arc::clone(&core));
        let state = || core.states.get(&entity).map(|s| s.state().to_owned());
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(state().as_deref(), Some("idle"));
        core.states
            .set(garage.clone(), "open", Attributes::new())
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(state().as_deref(), Some("on"));
        assert!(
            core.states
                .get(&entity)
                .unwrap()
                .attributes()
                .contains_key("prochain_rappel")
        );
        sent_messages(&mut rx);
        tokio::time::sleep(mins(10)).await;
        assert_eq!(sent_messages(&mut rx), ["Le garage est open."]);
        // « Compris », depuis le bouton de la notification.
        let mut data = Attributes::new();
        data.insert("action".into(), "RAPPEL_garage_ouvert".into());
        data.insert("user".into(), "david".into());
        core.fire(ACTION_EVENT, data).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let current = core.states.get(&entity).unwrap();
        assert_eq!(current.state(), "off");
        assert_eq!(current.attributes()["compris_par"], "david");
        tokio::time::sleep(mins(60)).await;
        assert_eq!(sent_messages(&mut rx), Vec::<String>::new(), "silence");
        // Fermé : le message de fin, et le rappel redevient calme.
        core.states
            .set(garage, "closed", Attributes::new())
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(sent_messages(&mut rx), ["Garage fermé."]);
        assert_eq!(state().as_deref(), Some("idle"));
        // Un rappel sans « Compris » refuse alert.turn_off.
        let freezer: EntityId = "binary_sensor.congelateur_chaud".parse().unwrap();
        core.states.set(freezer, "on", Attributes::new()).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let alert: EntityId = "alert.congelateur".parse().unwrap();
        core.call_service("alert", "turn_off", alert.clone(), Attributes::new())
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(core.states.get(&alert).unwrap().state(), "on");
        service.stop().await;
    }
}
