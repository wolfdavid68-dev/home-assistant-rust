//! Ce qu'on obtient, vérifié : des [`Automation`] prêtes à l'emploi, où plus
//! rien ne peut être invalide.

use std::time::Duration;

use ha_core::{Attributes, EntityId};
use ha_sun::SunEvent;
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};

use crate::template::Template;

use super::data::DataTemplates;
use super::read::RunMode;

/// Une automatisation vérifiée.
#[derive(Debug, Clone, PartialEq)]
pub struct Automation {
    pub id: String,
    pub alias: String,
    pub triggers: Vec<Trigger>,
    /// L'identifiant de chaque déclencheur (son `id`, ou sa position :
    /// `"0"`, `"1"`… comme dans Home Assistant).
    pub trigger_ids: Vec<String>,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
    pub mode: RunMode,
    /// Exécutions en attente ou simultanées au plus (`queued`, `parallel`).
    pub max: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Trigger {
    /// Une entité (ou l'un de ses attributs) change de valeur (`from` →
    /// `to`), éventuellement depuis `for`. Les listes vides ne filtrent rien.
    State {
        entity_ids: Vec<EntityId>,
        attribute: Option<String>,
        from: Vec<String>,
        to: Vec<String>,
        not_from: Vec<String>,
        not_to: Vec<String>,
        for_: Option<Duration>,
    },
    /// Une valeur numérique **entre** dans l'intervalle `]above, below[`.
    NumericState {
        entity_ids: Vec<EntityId>,
        above: Option<f64>,
        below: Option<f64>,
    },
    /// Tous les jours à une heure précise (heure locale), ou à l'heure
    /// réglée dans une entrée `input_datetime` (l'heure du réveil).
    Time { at: Vec<TimeAt> },
    /// Au lever, au coucher du soleil… avec un décalage (`-00:30:00` :
    /// une demi-heure avant).
    Sun {
        event: SunEvent,
        offset: SignedDuration,
    },
    /// Un événement nommé (`timer.finished`…), dont les détails contiennent
    /// au moins `event_data` (`{entity_id: timer.lessive}`).
    Event {
        event_type: String,
        event_data: Attributes,
    },
    /// Chaque fois que l'heure correspond au motif, à la seconde `second`.
    TimePattern {
        hours: Pattern,
        minutes: Pattern,
        second: i8,
    },
    /// Le début (ou la fin) d'un rendez-vous de l'agenda, plus `offset`.
    Calendar {
        entity_id: EntityId,
        edge: CalendarEdge,
        offset: SignedDuration,
    },
}

/// L'heure d'un déclencheur `time`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeAt {
    /// `07:30`.
    Fixed(Time),
    /// `input_datetime.reveil` : l'heure (ou la date et l'heure) qu'elle contient.
    Entity(EntityId),
}

/// Le début ou la fin d'un rendez-vous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarEdge {
    Start,
    End,
}

impl CalendarEdge {
    /// Le nom du champ de l'attribut `events` de l'agenda.
    #[must_use]
    pub fn field(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }
}

/// Le décalage d'un déclencheur `calendar` : une semaine au plus, car l'agenda
/// ne publie que les rendez-vous de la semaine passée et du mois à venir.
pub(super) const MAX_CALENDAR_OFFSET: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Un morceau de motif horaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// `*` : toutes les valeurs.
    Any,
    /// `/15` : 0, 15, 30, 45.
    Every(i8),
    /// `7` : exactement 7.
    At(i8),
}

impl Pattern {
    #[must_use]
    pub fn matches(self, value: i8) -> bool {
        match self {
            Self::Any => true,
            Self::Every(step) => value % step == 0,
            Self::At(at) => value == at,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// Chaque entité (ou son attribut `attribute`) vaut l'une de ces valeurs.
    State {
        entity_ids: Vec<EntityId>,
        attribute: Option<String>,
        states: Vec<String>,
    },
    NumericState {
        entity_ids: Vec<EntityId>,
        above: Option<f64>,
        below: Option<f64>,
    },
    Time {
        after: Option<Time>,
        before: Option<Time>,
        weekdays: Vec<Weekday>,
    },
    /// Après / avant un événement du soleil (avec décalage).
    Sun {
        after: Option<(SunEvent, SignedDuration)>,
        before: Option<(SunEvent, SignedDuration)>,
    },
    And(Vec<Condition>),
    Or(Vec<Condition>),
    Not(Vec<Condition>),
    /// Lancée par l'un de ces déclencheurs.
    Trigger(Vec<String>),
    /// Un modèle qui vaut « vrai » : `{{ states('sensor.x') | float > 20 }}`
    /// (étape 64). Le texte d'origine est gardé pour l'historique.
    Template {
        source: String,
        template: Template,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Service {
        domain: String,
        service: String,
        entity_ids: Vec<EntityId>,
        data: Attributes,
        /// Les modèles (`{{ … }}`) trouvés dans `data`, calculés à chaque appel.
        templates: DataTemplates,
    },
    Delay(Duration),
    /// Si les conditions sont vraies, `then`, sinon `otherwise`.
    If {
        conditions: Vec<Condition>,
        then: Vec<Action>,
        otherwise: Vec<Action>,
    },
    /// La première option dont les conditions sont vraies, sinon `default`.
    Choose {
        options: Vec<(Vec<Condition>, Vec<Action>)>,
        default: Vec<Action>,
    },
    /// Arrête l'exécution si la condition est fausse.
    Condition(Condition),
    /// Répète des actions.
    Repeat {
        until: RepeatUntil,
        actions: Vec<Action>,
    },
    /// Attend qu'un des déclencheurs se produise (au plus `timeout`). Après
    /// l'attente dépassée : on continue, ou on s'arrête là.
    WaitForTrigger {
        triggers: Vec<Trigger>,
        timeout: Option<Duration>,
        continue_on_timeout: bool,
    },
}

/// Combien de fois répéter.
#[derive(Debug, Clone, PartialEq)]
pub enum RepeatUntil {
    /// Un nombre de fois.
    Count(u32),
    /// Tant que les conditions sont vraies (vérifiées avant chaque tour).
    While(Vec<Condition>),
    /// Jusqu'à ce que les conditions soient vraies (vérifiées après chaque tour).
    Until(Vec<Condition>),
}

/// Tours d'une répétition, au plus (aussi pour `while` et `until`, qui
/// pourraient sinon tourner sans fin).
pub const MAX_REPEAT: u32 = 100;
