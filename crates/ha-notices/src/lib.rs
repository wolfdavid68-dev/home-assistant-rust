//! Les notifications du tableau de bord, qui restent affichées jusqu'à ce
//! qu'on les ferme (les « persistent notifications » de Home Assistant), et
//! les **alertes de réparation** : des problèmes que le serveur détecte
//! lui-même (appareil qui ne répond plus, automatisation qui vise une entité
//! disparue…).
//!
//! Chaque notification est une entité `persistent_notification.<id>`, comme
//! dans les anciennes versions de Home Assistant : elle est donc gardée dans
//! la base et retrouvée après un redémarrage, sans rien de plus.
//!
//! ```yaml
//! - action: persistent_notification.create
//!   target:
//!     entity_id: persistent_notification.porte_garage
//!   data:
//!     title: Garage
//!     message: La porte du garage est ouverte depuis 30 minutes.
//! ```
//!
//! `persistent_notification.dismiss` la ferme. Une alerte de réparation
//! fermée est seulement **ignorée** : elle disparaît pour de bon quand le
//! problème est réglé.
//!
//! Les **piles faibles** (étape 61) ont aussi leur alerte : à 20 % ou moins,
//! jusqu'à ce que la pile soit changée.
//!
//! Les ordres retenus par la garde (voir `ha_core::guard`) s'affichent aussi
//! ici, en notifications « à valider » (`approval_<numéro>`) : les fermer
//! revient à refuser l'ordre.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{
    Attributes, Background, CancellationToken, Core, EntityId, Event, Health, State, StateMachine,
};
use jiff::Timestamp;
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

/// Le domaine des notifications.
pub const DOMAIN: &str = "persistent_notification";
/// Le début de l'identifiant des alertes de réparation : réservé au serveur.
pub const REPAIR_PREFIX: &str = "repair_";
/// Le début de l'identifiant des ordres à valider : réservé au serveur.
pub const APPROVAL_PREFIX: &str = "approval_";
/// Notifications (hors réparations) gardées au plus : une automatisation qui
/// boucle ne doit pas remplir le tableau de bord.
pub const MAX_NOTIFICATIONS: usize = 50;
/// Alertes de réparation au plus.
pub const MAX_REPAIRS: usize = 100;
const MAX_TITLE_CHARS: usize = 100;
const MAX_MESSAGE_CHARS: usize = 1_000;

/// L'état d'une notification affichée.
pub const NOTIFYING: &str = "notifying";
/// L'état d'une alerte de réparation ignorée.
pub const IGNORED: &str = "ignored";

/// Un appareil est signalé injoignable après ce délai.
pub const UNAVAILABLE_AFTER: Duration = Duration::from_hours(1);
const UNAVAILABLE_PREFIX: &str = "repair_unavailable_";
/// Une intégration (Hue, Somfy…) est signalée en panne après ce délai : un
/// redémarrage du pont ou du broker ne doit pas déclencher d'alerte.
pub const INTEGRATION_ALERT_AFTER: Duration = Duration::from_mins(5);
const INTEGRATION_PREFIX: &str = "repair_integration_";
/// Une pile est signalée faible à ce niveau (en %) ou en dessous…
pub const BATTERY_LOW: f64 = 20.0;
/// … et l'alerte reste jusqu'à ce qu'elle dépasse ce niveau (pile changée) :
/// une mesure qui hésite entre 20 et 21 % ne fait pas clignoter l'alerte.
pub const BATTERY_REPLACED: f64 = 30.0;
/// Presque vide : l'alerte devient grave (et sonne sur le téléphone).
pub const BATTERY_EMPTY: f64 = 5.0;
const BATTERY_PREFIX: &str = "repair_battery_";
/// Les appareils qui répondent lentement (étape 63).
const SLOW_PREFIX: &str = "repair_slow_";
/// Délai avant la première vérification : au démarrage, les appareils
/// mettent un peu de temps à se reconnecter.
const FIRST_CHECK: Duration = Duration::from_mins(2);
const CHECK_EVERY: Duration = Duration::from_mins(1);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NoticeError {
    #[error("{0} n'est pas une notification (domaine persistent_notification attendu)")]
    WrongDomain(String),
    #[error("les identifiants « repair_… » et « approval_… » sont réservés au serveur")]
    Reserved,
    #[error("le message est obligatoire")]
    NoMessage,
    #[error("trop de notifications ({MAX_NOTIFICATIONS} au plus) : ferme les anciennes")]
    TooMany,
    #[error("identifiant invalide : {0}")]
    InvalidId(String),
    #[error(transparent)]
    Core(#[from] ha_core::CoreError),
}

/// La gravité d'une alerte de réparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Une alerte de réparation, telle qu'un module la signale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// La fin de l'identifiant (après le préfixe du module) : `salon`.
    pub key: String,
    pub title: String,
    pub message: String,
    pub severity: Severity,
}

/// Garde au plus `max` caractères, sans caractères de contrôle (sauf le
/// retour à la ligne) : le texte vient d'un fichier ou d'un appareil.
fn clean(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn text<'a>(attributes: &'a Attributes, key: &str) -> Option<&'a str> {
    attributes.get(key).and_then(Value::as_str)
}

fn object_id(entity_id: &EntityId) -> &str {
    entity_id
        .as_str()
        .split_once('.')
        .map_or("", |(_, object)| object)
}

fn is_repair(state: &State) -> bool {
    text(state.attributes(), "kind") == Some("repair")
}

/// Les notifications existantes.
fn all_notices(states: &StateMachine) -> Vec<State> {
    states
        .all()
        .into_iter()
        .filter(|state| state.entity_id().domain() == DOMAIN)
        .collect()
}

fn attributes(title: &str, message: &str, kind: &str, created_at: &str) -> Attributes {
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), title.into());
    attributes.insert("title".into(), title.into());
    attributes.insert("message".into(), message.into());
    attributes.insert("kind".into(), kind.into());
    attributes.insert("created_at".into(), created_at.into());
    attributes
}

/// Affiche une notification (ou la remplace, si elle existe déjà).
///
/// `entity_id` doit être dans le domaine `persistent_notification`, et ne
/// pas commencer par `repair_`. Sans titre, le titre est « Notification ».
pub fn notify(
    states: &StateMachine,
    entity_id: &EntityId,
    title: Option<&str>,
    message: &str,
) -> Result<(), NoticeError> {
    if entity_id.domain() != DOMAIN {
        return Err(NoticeError::WrongDomain(entity_id.to_string()));
    }
    let object = object_id(entity_id);
    if object.starts_with(REPAIR_PREFIX) || object.starts_with(APPROVAL_PREFIX) {
        return Err(NoticeError::Reserved);
    }
    let message = clean(message, MAX_MESSAGE_CHARS);
    if message.is_empty() {
        return Err(NoticeError::NoMessage);
    }
    let title = clean(title.unwrap_or_default(), MAX_TITLE_CHARS);
    let title = if title.is_empty() {
        "Notification".to_owned()
    } else {
        title
    };
    let existing = states.get(entity_id);
    if existing.is_none() {
        let count = all_notices(states)
            .iter()
            .filter(|state| !is_repair(state))
            .count();
        if count >= MAX_NOTIFICATIONS {
            return Err(NoticeError::TooMany);
        }
    }
    // La même notification, de nouveau : rien ne change (pas d'événement).
    let created_at = existing
        .as_ref()
        .filter(|state| {
            text(state.attributes(), "title") == Some(title.as_str())
                && text(state.attributes(), "message") == Some(message.as_str())
        })
        .and_then(|state| text(state.attributes(), "created_at").map(str::to_owned))
        .unwrap_or_else(|| Timestamp::now().to_string());
    states.set(
        entity_id.clone(),
        NOTIFYING,
        attributes(&title, &message, "notification", &created_at),
    )?;
    Ok(())
}

/// Ferme une notification. Une alerte de réparation est seulement ignorée.
/// Renvoie `false` si elle n'existait pas.
pub fn dismiss(states: &StateMachine, entity_id: &EntityId) -> Result<bool, NoticeError> {
    if entity_id.domain() != DOMAIN {
        return Err(NoticeError::WrongDomain(entity_id.to_string()));
    }
    let Some(state) = states.get(entity_id) else {
        return Ok(false);
    };
    if is_repair(&state) {
        states.set(entity_id.clone(), IGNORED, state.attributes().clone())?;
    } else {
        states.remove(entity_id);
    }
    Ok(true)
}

/// Met à jour les alertes d'un module (toutes celles dont l'identifiant
/// commence par `prefix`, qui doit commencer par `repair_`) : celles de
/// `issues` sont affichées, les autres (problèmes réglés) disparaissent.
/// Une alerte ignorée le reste tant que le problème dure.
pub fn reconcile(states: &StateMachine, prefix: &str, issues: &[Issue]) {
    if !prefix.starts_with(REPAIR_PREFIX) {
        warn!("alertes : préfixe {prefix} refusé");
        return;
    }
    let notices = all_notices(states);
    for state in &notices {
        let id = object_id(state.entity_id());
        if id.starts_with(prefix)
            && !issues
                .iter()
                .any(|issue| id.strip_prefix(prefix) == Some(issue.key.as_str()))
        {
            info!("alerte réglée : {}", state.entity_id());
            states.remove(state.entity_id());
        }
    }
    let mut repairs = notices.iter().filter(|state| is_repair(state)).count();
    for issue in issues {
        let Ok(entity_id) = format!("{DOMAIN}.{prefix}{}", issue.key).parse::<EntityId>() else {
            warn!("alerte : identifiant invalide ({prefix}{})", issue.key);
            continue;
        };
        let existing = states.get(&entity_id);
        let created_at = existing
            .as_ref()
            .and_then(|state| text(state.attributes(), "created_at").map(str::to_owned))
            .unwrap_or_else(|| Timestamp::now().to_string());
        let value = match &existing {
            Some(state) if state.state() == IGNORED => IGNORED,
            Some(_) => NOTIFYING,
            None if repairs >= MAX_REPAIRS => continue,
            None => {
                repairs += 1;
                warn!("alerte : {} ({})", issue.title, issue.message);
                NOTIFYING
            }
        };
        let mut attributes = attributes(
            &clean(&issue.title, MAX_TITLE_CHARS),
            &clean(&issue.message, MAX_MESSAGE_CHARS),
            "repair",
            &created_at,
        );
        attributes.insert("severity".into(), issue.severity.name().into());
        if let Err(error) = states.set(entity_id, value, attributes) {
            warn!("alerte refusée : {error}");
        }
    }
}

/// Le service des notifications, et la surveillance des appareils injoignables.
#[derive(Debug)]
pub struct Notices(Background);

impl Notices {
    /// Démarre le service. L'abonnement au bus est fait avant de rendre la main.
    pub fn start(core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("les notifications", |token| {
            run(core, events, token)
        }))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(
    core: Arc<Core>,
    mut events: tokio::sync::broadcast::Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    let mut checks =
        tokio::time::interval_at(tokio::time::Instant::now() + FIRST_CHECK, CHECK_EVERY);
    checks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // Les ordres à valider d'avant un redémarrage sont perdus : leurs
    // notifications aussi.
    approvals(&core);
    loop {
        tokio::select! {
            () = token.cancelled() => break,
            received = events.recv() => match received {
                Ok(event) => {
                    // La rapidité des appareils (étape 63) : chaque ordre,
                    // puis le changement d'état qui y répond.
                    core.latencies.observe(&event, &core.states, std::time::Instant::now());
                    match event.as_ref() {
                    Event::CallService { domain, service, entity_id, data, .. }
                        if domain == DOMAIN =>
                    {
                        on_call(&core, service, entity_id, data);
                    }
                    Event::Fired { event_type, .. } if event_type == ha_core::guard::HELD_CHANGED => {
                        approvals(&core);
                    }
                    _ => {}
                    }
                }
                Err(RecvError::Lagged(missed)) => warn!("notifications : {missed} événements manqués"),
                Err(RecvError::Closed) => break,
            },
            _ = checks.tick() => {
                let now = SystemTime::now();
                // D'abord les intégrations en panne (leurs appareils passent
                // « indisponibles »), puis les alertes.
                core.check_integrations(now);
                // Les ordres jamais validés sont oubliés (leur notification
                // disparaît avec l'événement publié).
                core.expire_held();
                reconcile(&core.states, INTEGRATION_PREFIX, &integration_issues(&core, now));
                reconcile(&core.states, UNAVAILABLE_PREFIX, &unavailable(&core, now));
                reconcile(&core.states, BATTERY_PREFIX, &low_batteries(&core.states));
                reconcile(&core.states, SLOW_PREFIX, &slow_devices(&core));
            }
        }
    }
}

fn on_call(core: &Core, service: &str, entity_id: &EntityId, data: &Attributes) {
    let states = &core.states;
    // Fermer un ordre à valider, c'est le refuser.
    if service == "dismiss"
        && let Some(id) = object_id(entity_id)
            .strip_prefix(APPROVAL_PREFIX)
            .and_then(|id| id.parse().ok())
    {
        if let Some(order) = core.release(id) {
            info!(
                "garde : {}.{} sur {} refusé (notification fermée)",
                order.domain, order.service, order.entity_id
            );
        }
        return;
    }
    let result = match service {
        "create" => notify(
            states,
            entity_id,
            text(data, "title"),
            text(data, "message").unwrap_or_default(),
        ),
        "dismiss" => dismiss(states, entity_id).map(|_| ()),
        other => {
            warn!("notifications : service inconnu {other}");
            Ok(())
        }
    };
    if let Err(error) = result {
        warn!("notification refusée ({entity_id}) : {error}");
    }
}

/// Met les notifications « à valider » d'accord avec les ordres retenus.
fn approvals(core: &Core) {
    let held = core.held_orders();
    let states = &core.states;
    for state in all_notices(states) {
        let still_held = object_id(state.entity_id())
            .strip_prefix(APPROVAL_PREFIX)
            .map(|id| held.iter().any(|order| order.id.to_string() == id));
        if still_held == Some(false) {
            states.remove(state.entity_id());
        }
    }
    let zone = jiff::tz::TimeZone::system();
    for order in held {
        let Ok(entity_id) = format!("{DOMAIN}.{APPROVAL_PREFIX}{}", order.id).parse::<EntityId>()
        else {
            continue;
        };
        if states.get(&entity_id).is_some() {
            continue;
        }
        let who = order.origin.describe().map_or_else(
            || "Un ordre".to_owned(),
            |by| format!("Un ordre donné {by}"),
        );
        let message = format!(
            "{who} attend ta validation : {}. Il est retenu parce que {}. \
             Sans réponse, il sera oublié à {}.",
            order_summary(states, &order),
            order.reason,
            Timestamp::try_from(order.held_since + ha_core::guard::HELD_TTL).map_or_else(
                |_| "un moment".to_owned(),
                |t| t.to_zoned(zone.clone()).strftime("%H:%M").to_string()
            )
        );
        let created_at = Timestamp::try_from(order.held_since)
            .unwrap_or_else(|_| Timestamp::now())
            .to_string();
        let mut attributes = attributes(
            "Ordre à valider",
            &clean(&message, MAX_MESSAGE_CHARS),
            "approval",
            &created_at,
        );
        attributes.insert("approval_id".into(), order.id.into());
        info!("garde : {}", order.reason);
        if let Err(error) = states.set(entity_id, NOTIFYING, attributes) {
            warn!("ordre à valider : notification refusée ({error})");
        }
    }
}

/// Ce que fait un ordre retenu : « fermer Volet chambre ».
#[must_use]
pub fn order_summary(states: &StateMachine, order: &ha_core::guard::HeldOrder) -> String {
    describe_order(states, &order.domain, &order.service, &order.entity_id)
}

/// « fermer Volet chambre », d'après le service et l'appareil.
#[must_use]
pub fn describe_order(
    states: &StateMachine,
    domain: &str,
    service: &str,
    entity_id: &EntityId,
) -> String {
    let device = states
        .get(entity_id)
        .and_then(|state| text(state.attributes(), "friendly_name").map(str::to_owned))
        .unwrap_or_else(|| entity_id.to_string());
    format!("{} {device}", action(domain, service))
}

/// « allumer », « fermer »… ou le nom du service.
fn action(domain: &str, service: &str) -> String {
    match service {
        "turn_on" => "allumer".into(),
        "turn_off" => "éteindre".into(),
        "toggle" => "basculer".into(),
        "open_cover" => "ouvrir".into(),
        "close_cover" => "fermer".into(),
        "stop_cover" => "arrêter".into(),
        "set_cover_position" => "régler la position de".into(),
        "set_temperature" => "changer la consigne de".into(),
        _ => format!("{domain}.{service} sur"),
    }
}

/// « le 16/10 à 10:32 ».
fn when(time: SystemTime, zone: &jiff::tz::TimeZone) -> String {
    Timestamp::try_from(time).map_or_else(
        |_| "longtemps".into(),
        |t| {
            t.to_zoned(zone.clone())
                .strftime("le %d/%m à %H:%M")
                .to_string()
        },
    )
}

/// Les intégrations en panne depuis au moins [`INTEGRATION_ALERT_AFTER`] :
/// une seule alerte pour Hue, plutôt qu'une par lampe.
#[must_use]
pub fn integration_issues(core: &Core, now: SystemTime) -> Vec<Issue> {
    let zone = jiff::tz::TimeZone::system();
    core.integrations()
        .into_iter()
        .filter(|status| {
            status.health.is_down()
                && now
                    .duration_since(status.since)
                    .is_ok_and(|down| down >= INTEGRATION_ALERT_AFTER)
        })
        .map(|status| {
            let (error, retry_at, severity) = match &status.health {
                Health::Crashed { error, retry_at } => {
                    (format!("Elle {error}."), Some(*retry_at), Severity::Error)
                }
                Health::Retrying { error, retry_at } => (
                    format!("Erreur : {error}."),
                    Some(*retry_at),
                    Severity::Warning,
                ),
                _ => (
                    "Elle n'a pas encore réussi à se connecter.".to_owned(),
                    None,
                    Severity::Warning,
                ),
            };
            let retry = retry_at
                .map(|at| format!(" Nouvel essai {}.", when(at, &zone)))
                .unwrap_or_default();
            let devices = if status.unavailable && status.entities > 0 {
                format!(
                    " Ses {} appareils sont indisponibles en attendant.",
                    status.entities
                )
            } else {
                String::new()
            };
            Issue {
                key: status.id.to_owned(),
                title: format!("{} ne répond plus", status.name),
                message: format!(
                    "{} est en panne depuis {}. {error}{retry}{devices}",
                    status.name,
                    when(status.since, &zone)
                ),
                severity,
            }
        })
        .collect()
}

/// Les appareils qui ne répondent plus depuis au moins [`UNAVAILABLE_AFTER`].
/// Ceux d'une intégration en panne n'ont pas d'alerte à eux : celle de
/// l'intégration suffit.
#[must_use]
pub fn unavailable(core: &Core, now: SystemTime) -> Vec<Issue> {
    let zone = jiff::tz::TimeZone::system();
    let down: Vec<&str> = core
        .integrations()
        .into_iter()
        .filter(|status| status.unavailable)
        .map(|status| status.id)
        .collect();
    let states = &core.states;
    states
        .all()
        .into_iter()
        .filter(|state| {
            state.state() == "unavailable"
                && states
                    .owner_of(state.entity_id())
                    .is_none_or(|owner| !down.contains(&owner.as_str()))
                && state.entity_id().domain() != DOMAIN
                && now
                    .duration_since(state.last_changed())
                    .is_ok_and(|since| since >= UNAVAILABLE_AFTER)
        })
        .map(|state| {
            let entity_id = state.entity_id();
            let name = text(state.attributes(), "friendly_name").unwrap_or(entity_id.as_str());
            let since = when(state.last_changed(), &zone);
            Issue {
                key: entity_id.as_str().replace('.', "_"),
                title: format!("{name} ne répond plus"),
                message: format!(
                    "{name} ({entity_id}) est injoignable depuis {since}. Vérifie qu'il est \
                     allumé, branché, et à portée du réseau."
                ),
                severity: Severity::Warning,
            }
        })
        .collect()
}

/// Ce qu'un appareil dit de sa pile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Battery {
    /// Le niveau, en %.
    Level(f64),
    /// Un détecteur qui dit seulement « pile faible » (`binary_sensor` de
    /// classe `battery`) : `true` quand elle l'est.
    Low(bool),
}

/// La pile d'une entité, s'il y en a une :
/// - un capteur de classe `battery` (en %), comme ceux de Zigbee2MQTT ;
/// - un détecteur de classe `battery` (« pile faible » : `on`) ;
/// - l'attribut `battery_level` d'un autre appareil (une serrure…).
///
/// Un aspirateur se recharge tout seul : sa batterie ne compte pas.
#[must_use]
pub fn battery(state: &State) -> Option<Battery> {
    if matches!(state.state(), "unavailable" | "unknown") {
        return None;
    }
    let domain = state.entity_id().domain();
    let attributes = state.attributes();
    let is_battery = text(attributes, "device_class") == Some("battery");
    let level = |value: Option<f64>| {
        value
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
            .map(Battery::Level)
    };
    match domain {
        "sensor" if is_battery => level(state.state().trim().parse().ok()),
        "binary_sensor" if is_battery => match state.state() {
            "on" => Some(Battery::Low(true)),
            "off" => Some(Battery::Low(false)),
            _ => None,
        },
        "vacuum" | DOMAIN => None,
        _ => level(attributes.get("battery_level").and_then(Value::as_f64)),
    }
}

/// Les piles faibles. Une alerte déjà affichée reste jusqu'à ce que la pile
/// dépasse [`BATTERY_REPLACED`].
#[must_use]
pub fn low_batteries(states: &StateMachine) -> Vec<Issue> {
    states
        .all()
        .into_iter()
        .filter_map(|state| {
            let entity_id = state.entity_id();
            let key = entity_id.as_str().replace('.', "_");
            let shown = format!("{DOMAIN}.{BATTERY_PREFIX}{key}")
                .parse::<EntityId>()
                .is_ok_and(|id| states.get(&id).is_some());
            let name = text(state.attributes(), "friendly_name").unwrap_or(entity_id.as_str());
            let (title, message, severity) = match battery(&state)? {
                Battery::Level(level) if level <= BATTERY_EMPTY => (
                    format!("Pile presque vide : {name}"),
                    format!(
                        "{name} ({entity_id}) n'a plus que {level:.0} % de pile : \
                         l'appareil peut s'arrêter à tout moment. Change la pile."
                    ),
                    Severity::Error,
                ),
                Battery::Level(level)
                    if level <= BATTERY_LOW || (shown && level <= BATTERY_REPLACED) =>
                {
                    (
                        format!("Pile faible : {name}"),
                        format!(
                            "{name} ({entity_id}) : {level:.0} % de pile. Prévois une pile \
                             neuve."
                        ),
                        Severity::Warning,
                    )
                }
                Battery::Low(true) => (
                    format!("Pile faible : {name}"),
                    format!(
                        "{name} ({entity_id}) signale une pile faible. Prévois une pile neuve."
                    ),
                    Severity::Warning,
                ),
                _ => return None,
            };
            Some(Issue {
                key,
                title,
                message,
                severity,
            })
        })
        .collect()
}

/// Les appareils dont les 3 derniers ordres ont mis plus de 3 secondes à
/// aboutir (voir [`ha_core::latency`]). Souvent une pile qui faiblit, un
/// appareil Zigbee trop loin, ou un réseau Wi-Fi saturé.
#[must_use]
pub fn slow_devices(core: &Core) -> Vec<Issue> {
    core.latencies
        .slow_devices()
        .into_iter()
        .filter_map(|(entity_id, fastest)| {
            let state = core.states.get(&entity_id)?;
            let name = text(state.attributes(), "friendly_name").unwrap_or(entity_id.as_str());
            // « 3,5 s », à la française.
            let seconds = format!("{:.1}", fastest.as_secs_f64()).replace('.', ",");
            Some(Issue {
                key: entity_id.as_str().replace('.', "_"),
                title: format!("{name} répond lentement"),
                message: format!(
                    "Les {} derniers ordres envoyés à {name} ({entity_id}) ont tous mis plus de \
                     {seconds} s à aboutir. Vérifie sa pile, sa distance au relais le plus \
                     proche (Zigbee) ou au point d'accès (Wi-Fi).",
                    ha_core::latency::DEVICE_SAMPLES
                ),
                severity: Severity::Warning,
            })
        })
        .collect()
}

/// Les alertes affichées (non ignorées), pour un résumé.
#[must_use]
pub fn active_count(states: &StateMachine) -> usize {
    all_notices(states)
        .iter()
        .filter(|state| state.state() == NOTIFYING)
        .count()
}

/// Vérifie un identifiant de notification venu de l'extérieur.
pub fn notice_id(object_id: &str) -> Result<EntityId, NoticeError> {
    format!("{DOMAIN}.{object_id}")
        .parse()
        .map_err(|_| NoticeError::InvalidId(object_id.to_owned()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]

    use super::*;

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    fn issue(key: &str) -> Issue {
        Issue {
            key: key.into(),
            title: format!("Problème {key}"),
            message: "Quelque chose ne va pas.".into(),
            severity: Severity::Warning,
        }
    }

    #[test]
    fn a_notification_is_created_then_dismissed() {
        let core = Core::new();
        let garage = id("persistent_notification.garage");
        notify(
            &core.states,
            &garage,
            Some("Garage"),
            "La porte est \u{7}ouverte.",
        )
        .unwrap();
        let state = core.states.get(&garage).unwrap();
        assert_eq!(state.state(), NOTIFYING);
        assert_eq!(
            text(state.attributes(), "message"),
            Some("La porte est ouverte.")
        );
        assert_eq!(text(state.attributes(), "kind"), Some("notification"));
        assert!(dismiss(&core.states, &garage).unwrap());
        assert!(core.states.get(&garage).is_none());
        assert!(!dismiss(&core.states, &garage).unwrap());
    }

    #[test]
    fn mistakes_and_forgeries_are_refused() {
        let core = Core::new();
        let states = &core.states;
        assert_eq!(
            notify(states, &id("light.salon"), None, "x"),
            Err(NoticeError::WrongDomain("light.salon".into()))
        );
        assert_eq!(
            notify(states, &id("persistent_notification.repair_x"), None, "x"),
            Err(NoticeError::Reserved)
        );
        assert_eq!(
            notify(states, &id("persistent_notification.vide"), None, "  "),
            Err(NoticeError::NoMessage)
        );
        for n in 0..MAX_NOTIFICATIONS {
            notify(
                states,
                &id(&format!("persistent_notification.n{n}")),
                None,
                "x",
            )
            .unwrap();
        }
        assert_eq!(
            notify(states, &id("persistent_notification.de_trop"), None, "x"),
            Err(NoticeError::TooMany)
        );
        // Remplacer une notification existante reste possible.
        notify(
            states,
            &id("persistent_notification.n0"),
            Some("Titre"),
            "y",
        )
        .unwrap();
        let long = "é".repeat(5_000);
        notify(
            states,
            &id("persistent_notification.n1"),
            Some(&long),
            &long,
        )
        .unwrap();
        let state = states.get(&id("persistent_notification.n1")).unwrap();
        assert_eq!(
            text(state.attributes(), "title").unwrap().chars().count(),
            MAX_TITLE_CHARS
        );
        assert_eq!(
            text(state.attributes(), "message").unwrap().chars().count(),
            MAX_MESSAGE_CHARS
        );
    }

    #[test]
    fn repairs_follow_the_problems() {
        let core = Core::new();
        let states = &core.states;
        reconcile(states, "repair_test_", &[issue("a"), issue("b")]);
        let a = id("persistent_notification.repair_test_a");
        let b = id("persistent_notification.repair_test_b");
        assert_eq!(states.get(&a).unwrap().state(), NOTIFYING);
        assert_eq!(active_count(states), 2);

        // Ignorée : elle reste, mais n'est plus affichée…
        assert!(dismiss(states, &a).unwrap());
        assert_eq!(states.get(&a).unwrap().state(), IGNORED);
        reconcile(states, "repair_test_", &[issue("a"), issue("b")]);
        assert_eq!(states.get(&a).unwrap().state(), IGNORED);
        assert_eq!(active_count(states), 1);

        // … et disparaît quand le problème est réglé. Les alertes des autres
        // modules ne sont pas touchées.
        reconcile(states, "repair_autre_", &[issue("c")]);
        reconcile(states, "repair_test_", &[issue("b")]);
        assert!(states.get(&a).is_none());
        assert!(states.get(&b).is_some());
        assert!(
            states
                .get(&id("persistent_notification.repair_autre_c"))
                .is_some()
        );

        // Un préfixe qui ne commence pas par « repair_ » ne peut rien effacer.
        notify(states, &id("persistent_notification.garage"), None, "x").unwrap();
        reconcile(states, "gar", &[]);
        assert!(states.get(&id("persistent_notification.garage")).is_some());
    }

    #[test]
    fn devices_that_stopped_answering_are_found() {
        let core = Core::new();
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), "Lampe du salon".into());
        core.states
            .set(id("light.salon"), "unavailable", attributes)
            .unwrap();
        core.states
            .set(id("light.cuisine"), "on", Attributes::new())
            .unwrap();
        let now = SystemTime::now();
        assert_eq!(unavailable(&core, now), Vec::<Issue>::new());
        let later = now + UNAVAILABLE_AFTER + Duration::from_secs(1);
        let issues = unavailable(&core, later);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].key, "light_salon");
        assert_eq!(issues[0].title, "Lampe du salon ne répond plus");
        assert!(issues[0].message.contains("(light.salon)"));
    }

    #[test]
    fn low_batteries_are_found_until_replaced() {
        let core = Core::new();
        let states = &core.states;
        let set = |entity: &str, value: &str, class: Option<&str>, level: Option<f64>| {
            let mut attributes = Attributes::new();
            attributes.insert("friendly_name".into(), format!("Appareil {entity}").into());
            if let Some(class) = class {
                attributes.insert("device_class".into(), class.into());
            }
            if let Some(level) = level {
                attributes.insert("battery_level".into(), level.into());
            }
            states.set(id(entity), value, attributes).unwrap();
        };
        let keys = || {
            let issues = low_batteries(states);
            reconcile(states, BATTERY_PREFIX, &issues);
            issues
                .into_iter()
                .map(|issue| (issue.key, issue.severity))
                .collect::<Vec<_>>()
        };
        set("sensor.fumee_batterie", "18", Some("battery"), None);
        set("sensor.porte_batterie", "64", Some("battery"), None);
        set("sensor.cave_batterie", "3", Some("battery"), None);
        set("binary_sensor.sonnette_pile", "on", Some("battery"), None);
        set("binary_sensor.fenetre_pile", "off", Some("battery"), None);
        set("lock.entree", "locked", None, Some(12.0));
        // Un aspirateur se recharge seul ; une valeur absurde ou une
        // température n'est pas une pile.
        set("vacuum.robot", "docked", None, Some(4.0));
        set("sensor.absurde", "180", Some("battery"), None);
        set("sensor.temperature", "12", Some("temperature"), None);
        set("sensor.muet", "unavailable", Some("battery"), None);
        let mut found = keys();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            found,
            [
                ("binary_sensor_sonnette_pile".into(), Severity::Warning),
                ("lock_entree".into(), Severity::Warning),
                ("sensor_cave_batterie".into(), Severity::Error),
                ("sensor_fumee_batterie".into(), Severity::Warning),
            ]
        );
        let alert = states
            .get(&id(
                "persistent_notification.repair_battery_sensor_fumee_batterie",
            ))
            .unwrap();
        assert_eq!(
            text(alert.attributes(), "title"),
            Some("Pile faible : Appareil sensor.fumee_batterie")
        );
        assert!(
            text(alert.attributes(), "message")
                .unwrap()
                .contains("18 %")
        );

        // 22 % : l'alerte déjà affichée reste (la mesure hésite) ; une pile
        // à 22 % sans alerte n'en fait pas apparaître.
        set("sensor.fumee_batterie", "22", Some("battery"), None);
        set("sensor.porte_batterie", "22", Some("battery"), None);
        let found = keys();
        assert!(found.iter().any(|(key, _)| key == "sensor_fumee_batterie"));
        assert!(!found.iter().any(|(key, _)| key == "sensor_porte_batterie"));
        // Pile changée : l'alerte disparaît.
        set("sensor.fumee_batterie", "100", Some("battery"), None);
        assert!(!keys().iter().any(|(key, _)| key == "sensor_fumee_batterie"));
        assert!(
            states
                .get(&id(
                    "persistent_notification.repair_battery_sensor_fumee_batterie"
                ))
                .is_none()
        );
    }

    #[test]
    fn a_device_that_answers_slowly_gets_an_alert() {
        let core = Core::new();
        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), "Prise de la cave".into());
        core.states
            .set_by("mqtt", id("switch.cave"), "off", attributes)
            .unwrap();
        let start = std::time::Instant::now();
        for (n, took_ms) in [(0_u64, 4_200_u64), (1, 3_600), (2, 5_000)] {
            let sent = start + Duration::from_secs(n * 10);
            let order = Event::CallService {
                domain: "switch".into(),
                service: "toggle".into(),
                entity_id: id("switch.cave"),
                data: Attributes::new(),
                origin: ha_core::Origin::System,
                command: None,
                issued_at: sent,
            };
            core.latencies.observe(&order, &core.states, sent);
            let answer = Event::StateChanged {
                entity_id: id("switch.cave"),
                old_state: None,
                new_state: core.states.get(&id("switch.cave")),
            };
            core.latencies
                .observe(&answer, &core.states, sent + Duration::from_millis(took_ms));
        }
        let issues = slow_devices(&core);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].key, "switch_cave");
        assert_eq!(issues[0].title, "Prise de la cave répond lentement");
        assert!(
            issues[0].message.contains("plus de 3,6 s"),
            "{}",
            issues[0].message
        );
        assert!(issues[0].message.contains("(switch.cave)"));
    }

    #[test]
    fn a_broken_integration_gets_one_alert_not_one_per_device() {
        let core = Core::new();
        core.report("hue", "Hue", Health::Connected);
        for name in ["salon", "cuisine", "chambre"] {
            core.states
                .set_by("hue", id(&format!("light.{name}")), "on", Attributes::new())
                .unwrap();
        }
        let now = SystemTime::now();
        core.report(
            "hue",
            "Hue",
            Health::Retrying {
                error: "pont injoignable".into(),
                retry_at: now,
            },
        );
        assert_eq!(integration_issues(&core, now), Vec::<Issue>::new());
        let later = now + INTEGRATION_ALERT_AFTER + Duration::from_secs(1);
        core.check_integrations(later);
        let issues = integration_issues(&core, later);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].key, "hue");
        assert_eq!(issues[0].title, "Hue ne répond plus");
        assert!(
            issues[0].message.contains("pont injoignable"),
            "{}",
            issues[0].message
        );
        assert!(
            issues[0].message.contains("Ses 3 appareils"),
            "{}",
            issues[0].message
        );
        // Les lampes sont indisponibles, mais sans alerte chacune.
        assert_eq!(
            core.states.get(&id("light.salon")).unwrap().state(),
            "unavailable"
        );
        assert_eq!(
            unavailable(&core, later + UNAVAILABLE_AFTER * 2),
            Vec::<Issue>::new()
        );
    }

    #[tokio::test]
    async fn the_service_answers_on_the_bus() {
        let core = Arc::new(Core::new());
        let notices = Notices::start(Arc::clone(&core));
        let mut data = Attributes::new();
        data.insert("message".into(), "Bonjour".into());
        let hello = id("persistent_notification.bonjour");
        core.call_service(DOMAIN, "create", hello.clone(), data)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(core.states.get(&hello).unwrap().state(), NOTIFYING);
        core.call_service(DOMAIN, "dismiss", hello.clone(), Attributes::new())
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(core.states.get(&hello).is_none());
        notices.stop().await;
    }

    #[tokio::test]
    async fn held_orders_are_shown_until_answered() {
        let core = Arc::new(Core::new());
        // Une notification « à valider » d'avant un redémarrage disparaît.
        let leftover = id("persistent_notification.approval_99");
        core.states
            .set(leftover.clone(), NOTIFYING, Attributes::new())
            .unwrap();
        let mut lamp = Attributes::new();
        lamp.insert("friendly_name".into(), "Lampe de chevet".into());
        core.states.set(id("light.chambre"), "off", lamp).unwrap();
        let notices = Notices::start(Arc::clone(&core));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(core.states.get(&leftover).is_none());

        let held = core
            .hold(
                ha_core::Origin::Voice,
                "light",
                "turn_on",
                id("light.chambre"),
                Attributes::new(),
                "« Chambre » est une pièce protégée".into(),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let notice = id(&format!("persistent_notification.approval_{held}"));
        let state = core.states.get(&notice).unwrap();
        assert_eq!(text(state.attributes(), "kind"), Some("approval"));
        assert_eq!(state.attributes()["approval_id"], held);
        let message = text(state.attributes(), "message").unwrap();
        assert!(
            message.starts_with(
                "Un ordre donné à la voix (Alexa) attend ta validation : allumer \
                 Lampe de chevet. Il est retenu parce que « Chambre » est une pièce protégée."
            ),
            "{message}"
        );
        // Personne ne peut fabriquer une fausse demande de validation.
        assert_eq!(
            notify(
                &core.states,
                &id("persistent_notification.approval_7"),
                None,
                "x"
            ),
            Err(NoticeError::Reserved)
        );

        // Fermer la notification refuse l'ordre.
        core.call_service(DOMAIN, "dismiss", notice.clone(), Attributes::new())
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(core.held_orders().len(), 0);
        assert!(core.states.get(&notice).is_none());
        notices.stop().await;
    }
}
