//! La **garde** : des pièces protégées et des heures calmes.
//!
//! Alexa peut allumer la lumière de la chambre à 3 h du matin parce que la
//! télévision a prononcé « allume la chambre » ; un invité peut couper le
//! chauffage du bureau. La garde retient ces ordres-là :
//!
//! - une **pièce protégée** (la chambre, le bureau) ne se commande pas à la
//!   voix ni par un invité sans qu'un membre de la famille valide ;
//! - pendant les **heures calmes** (22:00 – 07:00), la protection ne vaut
//!   que la nuit ; sans pièce protégée, elle vaut pour toute la maison.
//!
//! Un ordre retenu attend ([`HELD_TTL`] au plus) qu'on le valide ou qu'on le
//! refuse ; une notification « Valider ? » s'affiche dans la cloche. Un clic
//! dans le tableau de bord d'un membre de la famille n'est jamais retenu :
//! c'est lui qui décide.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::{Attributes, Core, CoreError, EntityId, Origin, check_service_names};

/// Un ordre retenu sans réponse est oublié après ce délai : l'invité qui
/// voulait allumer la lumière est parti depuis longtemps.
pub const HELD_TTL: Duration = Duration::from_mins(10);
/// Ordres retenus au plus : au-delà, le plus ancien est oublié.
pub const MAX_HELD: usize = 20;
/// L'événement publié quand la liste des ordres retenus change.
pub const HELD_CHANGED: &str = "guard.held_changed";
/// Un ordre retenu a été validé ou refusé : `{id, domain, service,
/// entity_id, approved, by}`. Seul le serveur le publie.
pub const APPROVAL_ANSWERED: &str = "guard.approval_answered";
/// Un ordre retenu a été oublié sans réponse : `{id, domain, service,
/// entity_id}`.
pub const APPROVAL_EXPIRED: &str = "guard.approval_expired";

/// Les heures calmes, en minutes depuis minuit (heure locale). `from` peut
/// être après `to` : 22:00 – 07:00 passe minuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietHours {
    pub from: u16,
    pub to: u16,
}

impl QuietHours {
    /// Cette minute (0 à 1439) est-elle dans les heures calmes ?
    #[must_use]
    pub fn contains(self, minute: u16) -> bool {
        if self.from <= self.to {
            (self.from..self.to).contains(&minute)
        } else {
            minute >= self.from || minute < self.to
        }
    }

    /// « 22:00 – 07:00 ».
    #[must_use]
    pub fn describe(self) -> String {
        format!("{} – {}", hhmm(self.from), hhmm(self.to))
    }
}

fn hhmm(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// Ce que la garde protège (lu dans `guard.yaml`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Les appareils protégés, avec le nom de leur pièce (« Chambre »).
    pub protected: HashMap<EntityId, String>,
    pub quiet_hours: Option<QuietHours>,
}

impl Policy {
    /// La garde ne fait rien.
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.protected.is_empty() && self.quiet_hours.is_none()
    }

    /// Pourquoi un ordre sur cet appareil est retenu à cette minute de la
    /// journée, ou `None` s'il passe.
    #[must_use]
    pub fn check(&self, entity_id: &EntityId, minute: u16) -> Option<String> {
        let quiet = self.quiet_hours.filter(|hours| hours.contains(minute));
        match (self.protected.get(entity_id), self.quiet_hours) {
            (Some(area), None) => Some(format!("«\u{a0}{area}\u{a0}» est une pièce protégée")),
            (Some(area), Some(hours)) => quiet.map(|_| {
                format!(
                    "«\u{a0}{area}\u{a0}» est protégée pendant les heures calmes ({})",
                    hours.describe()
                )
            }),
            (None, _) if self.protected.is_empty() => {
                quiet.map(|hours| format!("ce sont les heures calmes ({})", hours.describe()))
            }
            (None, _) => None,
        }
    }
}

/// Un ordre qui attend d'être validé.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldOrder {
    pub id: u64,
    pub domain: String,
    pub service: String,
    pub entity_id: EntityId,
    pub data: Attributes,
    pub origin: Origin,
    /// Pourquoi il est retenu (« « Chambre » est une pièce protégée »).
    pub reason: String,
    pub held_since: SystemTime,
    held_at: Instant,
}

/// La garde : la règle, l'heure locale, et les ordres retenus.
#[derive(Debug)]
pub struct Guard {
    policy: RwLock<Policy>,
    /// Donne la minute de la journée en heure locale : le noyau ne connaît
    /// pas les fuseaux horaires, le serveur la fournit.
    clock: RwLock<fn() -> u16>,
    next: AtomicU64,
    held: Mutex<VecDeque<HeldOrder>>,
}

impl Default for Guard {
    fn default() -> Self {
        Self {
            policy: RwLock::default(),
            clock: RwLock::new(utc_minute),
            next: AtomicU64::default(),
            held: Mutex::default(),
        }
    }
}

/// La minute de la journée en temps universel (à défaut d'heure locale).
fn utc_minute() -> u16 {
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    u16::try_from(seconds % 86_400 / 60).unwrap_or(0)
}

impl Guard {
    /// Installe la règle, et l'horloge qui donne la minute locale.
    pub fn configure(&self, policy: Policy, clock: fn() -> u16) {
        *self.policy.write().unwrap_or_else(PoisonError::into_inner) = policy;
        *self.clock.write().unwrap_or_else(PoisonError::into_inner) = clock;
    }

    /// Une copie de la règle en place.
    #[must_use]
    pub fn policy(&self) -> Policy {
        self.policy
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn check(&self, entity_id: &EntityId) -> Option<String> {
        let policy = self.policy.read().unwrap_or_else(PoisonError::into_inner);
        if policy.is_off() {
            return None;
        }
        let minute = (self.clock.read().unwrap_or_else(PoisonError::into_inner))();
        policy.check(entity_id, minute)
    }

    fn held(&self) -> std::sync::MutexGuard<'_, VecDeque<HeldOrder>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Core {
    /// Pourquoi un ordre venant de la voix ou d'un invité sur cet appareil
    /// doit être retenu, ou `None` s'il passe. Pour un groupe, chacun de ses
    /// membres compte : un invité ne doit pas allumer la chambre en passant
    /// par « toutes les lumières ».
    #[must_use]
    pub fn guard_reason(&self, entity_id: &EntityId) -> Option<String> {
        if let Some(reason) = self.guard.check(entity_id) {
            return Some(reason);
        }
        let state = self.states.get(entity_id)?;
        let members = state.attributes().get("entity_id")?.as_array()?;
        members
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|id| id.parse::<EntityId>().ok())
            .find_map(|member| self.guard.check(&member))
    }

    /// Retient un ordre en attendant qu'on le valide. Renvoie son numéro.
    pub fn hold(
        &self,
        origin: Origin,
        domain: &str,
        service: &str,
        entity_id: EntityId,
        data: Attributes,
        reason: String,
    ) -> Result<u64, CoreError> {
        check_service_names(domain, service)?;
        let id = self.guard.next.fetch_add(1, Ordering::Relaxed) + 1;
        let dropped = {
            let mut held = self.guard.held();
            let dropped = if held.len() >= MAX_HELD {
                held.pop_front()
            } else {
                None
            };
            held.push_back(HeldOrder {
                id,
                domain: domain.to_owned(),
                service: service.to_owned(),
                entity_id,
                data,
                origin,
                reason,
                held_since: SystemTime::now(),
                held_at: Instant::now(),
            });
            dropped
        };
        if let Some(order) = dropped {
            self.announce(APPROVAL_EXPIRED, &order, Attributes::new());
        }
        self.held_changed();
        Ok(id)
    }

    /// Les ordres qui attendent, du plus ancien au plus récent.
    #[must_use]
    pub fn held_orders(&self) -> Vec<HeldOrder> {
        self.guard.held().iter().cloned().collect()
    }

    /// Retire un ordre de l'attente (pour le valider ou le refuser).
    /// `None` s'il n'existe pas, ou plus.
    pub fn release(&self, id: u64) -> Option<HeldOrder> {
        let order = {
            let mut held = self.guard.held();
            let index = held.iter().position(|order| order.id == id)?;
            held.remove(index)
        };
        self.held_changed();
        order
    }

    /// Oublie les ordres qui attendent depuis plus de [`HELD_TTL`]. Renvoie
    /// combien.
    pub fn expire_held(&self) -> usize {
        let expired: Vec<HeldOrder> = {
            let mut held = self.guard.held();
            let (kept, expired): (VecDeque<HeldOrder>, VecDeque<HeldOrder>) = held
                .drain(..)
                .partition(|order| order.held_at.elapsed() < HELD_TTL);
            *held = kept;
            expired.into_iter().collect()
        };
        for order in &expired {
            self.announce(APPROVAL_EXPIRED, order, Attributes::new());
        }
        if !expired.is_empty() {
            tracing::info!("garde : {} ordre(s) non validé(s) oublié(s)", expired.len());
            self.held_changed();
        }
        expired.len()
    }

    /// Annonce qu'un ordre retenu a été validé ou refusé, et par qui (les
    /// téléphones remplacent alors leur notification « Ordre à valider »).
    pub fn approval_answered(&self, order: &HeldOrder, approved: bool, by: &str) {
        let mut extra = Attributes::new();
        extra.insert("approved".into(), approved.into());
        extra.insert("by".into(), by.into());
        self.announce(APPROVAL_ANSWERED, order, extra);
    }

    fn announce(&self, event_type: &str, order: &HeldOrder, mut data: Attributes) {
        data.insert("id".into(), order.id.into());
        data.insert("domain".into(), order.domain.clone().into());
        data.insert("service".into(), order.service.clone().into());
        data.insert("entity_id".into(), order.entity_id.as_str().into());
        // Les noms sont valides : l'erreur est impossible.
        let _ = self.fire(event_type, data);
    }

    fn held_changed(&self) {
        // Le nom est valide : l'erreur est impossible.
        let _ = self.fire(HELD_CHANGED, Attributes::new());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use crate::Event;

    fn id(text: &str) -> EntityId {
        text.parse().unwrap()
    }

    const NIGHT: QuietHours = QuietHours {
        from: 22 * 60,
        to: 7 * 60,
    };

    fn bedroom(quiet_hours: Option<QuietHours>) -> Policy {
        Policy {
            protected: HashMap::from([(id("light.chambre"), "Chambre".to_owned())]),
            quiet_hours,
        }
    }

    #[test]
    fn quiet_hours_can_cross_midnight() {
        assert!(NIGHT.contains(23 * 60));
        assert!(NIGHT.contains(0));
        assert!(NIGHT.contains(6 * 60 + 59));
        assert!(!NIGHT.contains(7 * 60));
        assert!(!NIGHT.contains(12 * 60));
        let lunch = QuietHours {
            from: 12 * 60,
            to: 14 * 60,
        };
        assert!(lunch.contains(13 * 60));
        assert!(!lunch.contains(23 * 60));
        assert_eq!(NIGHT.describe(), "22:00 – 07:00");
    }

    #[test]
    fn a_protected_room_is_always_guarded_without_quiet_hours() {
        let policy = bedroom(None);
        assert_eq!(
            policy.check(&id("light.chambre"), 12 * 60).unwrap(),
            "«\u{a0}Chambre\u{a0}» est une pièce protégée"
        );
        assert!(policy.check(&id("light.salon"), 12 * 60).is_none());
        assert!(Policy::default().is_off());
    }

    #[test]
    fn with_quiet_hours_a_protected_room_is_guarded_at_night_only() {
        let policy = bedroom(Some(NIGHT));
        assert!(policy.check(&id("light.chambre"), 12 * 60).is_none());
        assert!(
            policy
                .check(&id("light.chambre"), 23 * 60)
                .unwrap()
                .contains("heures calmes (22:00 – 07:00)")
        );
        // Le salon n'est pas protégé, même la nuit.
        assert!(policy.check(&id("light.salon"), 23 * 60).is_none());
    }

    #[test]
    fn quiet_hours_alone_guard_the_whole_house_at_night() {
        let policy = Policy {
            protected: HashMap::new(),
            quiet_hours: Some(NIGHT),
        };
        assert!(policy.check(&id("light.salon"), 23 * 60).is_some());
        assert!(policy.check(&id("light.salon"), 9 * 60).is_none());
    }

    #[test]
    fn a_group_containing_a_protected_device_is_guarded() {
        let core = Core::new();
        core.guard.configure(bedroom(None), || 0);
        let mut members = Attributes::new();
        members.insert(
            "entity_id".into(),
            serde_json::json!(["light.salon", "light.chambre"]),
        );
        core.states.set(id("light.tout"), "on", members).unwrap();
        assert!(core.guard_reason(&id("light.tout")).is_some());
        assert!(core.guard_reason(&id("light.salon")).is_none());
    }

    #[tokio::test]
    async fn held_orders_wait_for_an_answer() {
        let core = Core::new();
        let mut events = core.bus.subscribe();
        let first = core
            .hold(
                Origin::Voice,
                "light",
                "turn_on",
                id("light.chambre"),
                Attributes::new(),
                "protégée".into(),
            )
            .unwrap();
        let Event::Fired { event_type, .. } = events.recv().await.unwrap().as_ref().clone() else {
            panic!("un événement était attendu");
        };
        assert_eq!(event_type, HELD_CHANGED);
        assert_eq!(core.held_orders().len(), 1);
        let order = core.release(first).unwrap();
        assert_eq!(order.origin, Origin::Voice);
        assert!(core.release(first).is_none());
        assert_eq!(core.held_orders().len(), 0);
        // Un nom de service invalide n'est pas retenu.
        assert!(
            core.hold(
                Origin::Voice,
                "Light",
                "x",
                id("light.chambre"),
                Attributes::new(),
                String::new()
            )
            .is_err()
        );
    }

    #[test]
    fn only_the_latest_held_orders_are_kept() {
        let core = Core::new();
        for _ in 0..=MAX_HELD {
            core.hold(
                Origin::Voice,
                "light",
                "turn_on",
                id("light.chambre"),
                Attributes::new(),
                String::new(),
            )
            .unwrap();
        }
        let held = core.held_orders();
        assert_eq!(held.len(), MAX_HELD);
        assert_eq!(held.first().unwrap().id, 2);
        assert_eq!(core.expire_held(), 0);
    }

    #[test]
    fn answered_and_forgotten_orders_are_announced() {
        let core = Core::new();
        let mut events = core.bus.subscribe();
        let announced = |event: &Event| match event {
            Event::Fired { event_type, data } if event_type != HELD_CHANGED => {
                Some((event_type.clone(), data.clone()))
            }
            _ => None,
        };
        // Le premier ordre est poussé dehors par le 21e : oublié.
        for _ in 0..=MAX_HELD {
            core.hold(
                Origin::Voice,
                "cover",
                "close_cover",
                id("cover.chambre"),
                Attributes::new(),
                String::new(),
            )
            .unwrap();
        }
        let mut forgotten = None;
        while let Ok(event) = events.try_recv() {
            if let Some(found) = announced(&event) {
                forgotten = Some(found);
            }
        }
        let (event_type, data) = forgotten.unwrap();
        assert_eq!(event_type, APPROVAL_EXPIRED);
        assert_eq!(data["id"], 1);
        assert_eq!(data["service"], "close_cover");
        assert_eq!(data["entity_id"], "cover.chambre");

        let order = core.release(2).unwrap();
        core.approval_answered(&order, true, "alice");
        let (event_type, data) = std::iter::from_fn(|| events.try_recv().ok())
            .find_map(|event| announced(&event))
            .unwrap();
        assert_eq!(event_type, APPROVAL_ANSWERED);
        assert_eq!(data["id"], 2);
        assert_eq!(data["approved"], true);
        assert_eq!(data["by"], "alice");
    }
}
