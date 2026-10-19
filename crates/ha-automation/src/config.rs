//! Le fichier `automations.yaml` : lecture et vérification.
//!
//! Deux temps :
//! 1. **lecture** : le YAML est transformé en structures Rust (`*Config`), avec
//!    `deny_unknown_fields` pour refuser toute faute de frappe ;
//! 2. **vérification** : chaque valeur est contrôlée (identifiants d'entités,
//!    heures, durées, limites) et l'on obtient des [`Automation`] prêtes à
//!    l'emploi, où plus rien ne peut être invalide.

use std::collections::HashSet;
use std::time::Duration;

use ha_core::{Attributes, EntityId};
use ha_sun::SunEvent;
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AutomationError;
use crate::template::{Scope, Template};

/// Taille maximale du fichier.
pub const MAX_FILE_SIZE: usize = 1024 * 1024;
/// Nombre maximal d'automatisations, et d'éléments dans chacune.
pub const MAX_AUTOMATIONS: usize = 200;
const MAX_ITEMS: usize = 20;
/// Durée maximale d'une attente (`delay`, `for`).
const MAX_DURATION: Duration = Duration::from_hours(24);

// ───────────────────────────── 1. Ce qu'on lit ─────────────────────────────

/// Une valeur écrite comme du texte : `"on"`, `home`, `21`…
///
/// Un nombre sans guillemets (`state: 3`) est accepté et devient du texte,
/// comme dans Home Assistant. Mais `on`, `off`, `yes`, `true` sans guillemets
/// sont lus par YAML comme « vrai » ou « faux » : on les refuse avec un
/// message clair, plutôt que de deviner ce qui était voulu.
///
/// Concept Rust : on écrit ici la lecture « à la main », avec un `Visitor`.
/// Serde appelle la méthode qui correspond à ce qu'il a trouvé dans le
/// fichier (`visit_str` pour du texte, `visit_i64` pour un entier…).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct Text(String);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Text;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("du texte")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Text, E> {
                Ok(Text(value.to_owned()))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Text, E> {
                Ok(Text(value.to_string()))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Text, E> {
                Ok(Text(value.to_string()))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Text, E> {
                Ok(Text(value.to_string()))
            }

            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Text, E> {
                Err(E::custom(
                    "valeur lue comme vrai/faux : écris-la entre guillemets, par exemple \"on\" ou \"off\"",
                ))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Une valeur **ou** une liste : `entity_id: light.salon` ou
/// `to: [Orange, Rouge]`.
///
/// Concept Rust : `#[serde(untagged)]` saurait essayer les deux formes, mais
/// remplacerait le message d'erreur précis de [`Text`] (« écris-la entre
/// guillemets ») par un message vague. La lecture est donc écrite à la main :
/// une liste est lue élément par élément, le reste passe par `Text`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum OneOrMany {
    One(Text),
    Many(Vec<Text>),
}

impl<'de> Deserialize<'de> for OneOrMany {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = OneOrMany;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("du texte ou une liste")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<OneOrMany, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<Text>()? {
                    values.push(value);
                }
                Ok(OneOrMany::Many(values))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<OneOrMany, E> {
                Ok(OneOrMany::One(Text(value.to_owned())))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<OneOrMany, E> {
                Ok(OneOrMany::One(Text(value.to_string())))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<OneOrMany, E> {
                Ok(OneOrMany::One(Text(value.to_string())))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<OneOrMany, E> {
                Ok(OneOrMany::One(Text(value.to_string())))
            }

            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<OneOrMany, E> {
                Err(E::custom(
                    "valeur lue comme vrai/faux : écris-la entre guillemets, par exemple \"on\" ou \"off\"",
                ))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value.0],
            Self::Many(values) => values.into_iter().map(|value| value.0).collect(),
        }
    }
}

/// Une automatisation **telle qu'elle est écrite**, avant vérification.
///
/// Concept Rust : la même structure sait se lire (`Deserialize`) et s'écrire
/// (`Serialize`). Une automatisation lue en YAML, ou envoyée en JSON par le
/// tableau de bord, passe toujours par elle : elle est donc réécrite sous une
/// forme unique (« canonique »), quel que soit le format d'origine.
///
/// Les champs sont privés : on ne peut obtenir une [`Automation`] qu'en
/// passant par [`AutomationConfig::check`].
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationConfig {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    #[serde(default = "enabled_by_default", skip_serializing_if = "is_true")]
    enabled: bool,
    /// Que faire si elle se redéclenche pendant qu'elle tourne encore.
    #[serde(default, skip_serializing_if = "RunMode::is_single")]
    mode: RunMode,
    /// Nombre maximal d'exécutions en attente ou simultanées (`queued`, `parallel`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<u8>,
    triggers: Vec<TriggerConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conditions: Vec<ConditionConfig>,
    actions: Vec<ActionConfig>,
}

/// Que faire si une automatisation se redéclenche pendant qu'elle tourne
/// encore (par exemple pendant un `delay`). Comme dans Home Assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Ignorer le nouveau déclenchement (par défaut).
    #[default]
    Single,
    /// Arrêter l'exécution en cours et recommencer depuis le début.
    Restart,
    /// Attendre la fin de l'exécution en cours, puis recommencer.
    Queued,
    /// Lancer une nouvelle exécution en même temps.
    Parallel,
}

impl RunMode {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde passe toujours une référence"
    )]
    fn is_single(&self) -> bool {
        *self == Self::Single
    }
}

/// Exécutions en attente ou simultanées, par défaut et au plus.
const DEFAULT_MAX_RUNS: u8 = 10;
const MAX_MAX_RUNS: u8 = 20;

fn enabled_by_default() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passe toujours une référence"
)]
fn is_true(value: &bool) -> bool {
    *value
}

/// Concept Rust : `#[serde(tag = "trigger")]` choisit la variante d'après la
/// valeur du champ `trigger:` (`state`, `numeric_state` ou `time`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "trigger", rename_all = "snake_case", deny_unknown_fields)]
enum TriggerConfig {
    State {
        entity_id: OneOrMany,
        /// Surveiller un attribut (`Vent violent`) plutôt que l'état.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribute: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<OneOrMany>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<OneOrMany>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        not_from: Option<OneOrMany>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        not_to: Option<OneOrMany>,
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        for_: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    NumericState {
        entity_id: OneOrMany,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Time {
        at: OneOrMany,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Sun {
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Selon un motif horaire : `minutes: "/15"` (toutes les 15 minutes),
    /// `hours: 7` (chaque jour à 7 h 00)…
    TimePattern {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hours: Option<Text>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minutes: Option<Text>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<Text>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Un événement nommé : `timer.finished`, par exemple.
    Event {
        event_type: String,
        #[serde(default, skip_serializing_if = "Attributes::is_empty")]
        event_data: Attributes,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Le début ou la fin d'un rendez-vous de l'agenda, avec un décalage
    /// (`-00:30:00` : une demi-heure avant).
    Calendar {
        entity_id: String,
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "condition", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ConditionConfig {
    State {
        entity_id: OneOrMany,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribute: Option<String>,
        state: OneOrMany,
    },
    NumericState {
        entity_id: OneOrMany,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
    },
    Time {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weekday: Option<OneOrMany>,
    },
    Sun {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_offset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_offset: Option<String>,
    },
    /// Toutes les conditions de la liste.
    And { conditions: Vec<ConditionConfig> },
    /// Au moins une des conditions.
    Or { conditions: Vec<ConditionConfig> },
    /// Aucune des conditions.
    Not { conditions: Vec<ConditionConfig> },
    /// L'automatisation a été lancée par ce déclencheur (son `id`).
    Trigger { id: OneOrMany },
    /// Un modèle qui vaut « vrai » (étape 64).
    Template { value_template: String },
}

/// Une action. Concept Rust : la lecture est écrite à la main (voir
/// `Deserialize` plus bas) pour choisir la forme d'après la **clé** présente
/// (`action`, `delay`, `if`…) et donner un message d'erreur précis. L'écriture,
/// elle, est automatique (`untagged` : chaque forme s'écrit telle quelle).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum ActionConfig {
    Service(ServiceActionConfig),
    Delay(DelayActionConfig),
    If(IfActionConfig),
    Choose(ChooseActionConfig),
    /// « Continuer seulement si… » : arrête l'automatisation si c'est faux.
    Condition(ConditionConfig),
    Repeat(RepeatActionConfig),
    WaitForTrigger(WaitActionConfig),
}

/// `repeat:` avec `count`, `while` ou `until`, et `sequence`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepeatActionConfig {
    repeat: RepeatConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepeatConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    count: Option<u32>,
    #[serde(default, rename = "while", skip_serializing_if = "Option::is_none")]
    while_: Option<Vec<ConditionConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    until: Option<Vec<ConditionConfig>>,
    sequence: Vec<ActionConfig>,
}

/// `wait_for_trigger:` (déclencheurs), `timeout:` et `continue_on_timeout:`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaitActionConfig {
    wait_for_trigger: Vec<TriggerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    continue_on_timeout: Option<bool>,
}

impl<'de> Deserialize<'de> for ActionConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = Value::deserialize(deserializer)?;
        let has = |key: &str| value.get(key).is_some();
        let result = if has("action") || has("service") {
            serde_json::from_value(value).map(Self::Service)
        } else if has("delay") {
            serde_json::from_value(value).map(Self::Delay)
        } else if has("if") {
            serde_json::from_value(value).map(Self::If)
        } else if has("choose") {
            serde_json::from_value(value).map(Self::Choose)
        } else if has("repeat") {
            serde_json::from_value(value).map(Self::Repeat)
        } else if has("wait_for_trigger") {
            serde_json::from_value(value).map(Self::WaitForTrigger)
        } else if has("condition") {
            serde_json::from_value(value).map(Self::Condition)
        } else {
            return Err(D::Error::custom(
                "action inconnue : il faut « action », « delay », « if », « choose », « repeat », \
                 « wait_for_trigger » ou « condition »",
            ));
        };
        result.map_err(D::Error::custom)
    }
}

/// `if:` (conditions) `then:` (actions) `else:` (actions, facultatif).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IfActionConfig {
    #[serde(rename = "if")]
    if_: Vec<ConditionConfig>,
    then: Vec<ActionConfig>,
    #[serde(default, rename = "else", skip_serializing_if = "Option::is_none")]
    else_: Option<Vec<ActionConfig>>,
}

/// `choose:` une liste d'options (la première dont les conditions sont
/// vraies est exécutée), et `default:` si aucune ne l'est.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChooseActionConfig {
    choose: Vec<ChooseOptionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<Vec<ActionConfig>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChooseOptionConfig {
    conditions: Vec<ConditionConfig>,
    sequence: Vec<ActionConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceActionConfig {
    /// `light.turn_on` (Home Assistant accepte aussi l'ancien nom `service`).
    #[serde(alias = "service")]
    action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<TargetConfig>,
    #[serde(default, skip_serializing_if = "Attributes::is_empty")]
    data: Attributes,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetConfig {
    entity_id: OneOrMany,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DelayActionConfig {
    delay: String,
}

// ─────────────────────── 2. Ce qu'on obtient, vérifié ───────────────────────

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
const MAX_CALENDAR_OFFSET: SignedDuration = SignedDuration::from_hours(7 * 24);

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

/// Lit et vérifie le contenu d'un fichier `automations.yaml`. Les
/// automatisations désactivées sont vérifiées, mais pas renvoyées.
pub fn parse(yaml: &str) -> Result<Vec<Automation>, AutomationError> {
    check_all(&parse_configs(yaml)?)
}

/// Lit un fichier `automations.yaml` **sans** vérifier les valeurs : seule la
/// forme (champs connus, types) est contrôlée. Voir [`check_all`].
pub fn parse_configs(yaml: &str) -> Result<Vec<AutomationConfig>, AutomationError> {
    if yaml.len() > MAX_FILE_SIZE {
        return Err(AutomationError::TooLarge);
    }
    if yaml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let configs: Vec<AutomationConfig> =
        serde_saphyr::from_str(yaml).map_err(|error| AutomationError::Yaml(error.to_string()))?;
    if configs.len() > MAX_AUTOMATIONS {
        return Err(AutomationError::TooMany(MAX_AUTOMATIONS));
    }
    Ok(configs)
}

/// Vérifie une liste d'automatisations (identifiants uniques compris), et
/// renvoie celles qui sont activées.
pub fn check_all(configs: &[AutomationConfig]) -> Result<Vec<Automation>, AutomationError> {
    let mut ids = HashSet::new();
    let mut automations = Vec::new();
    for config in configs {
        if !ids.insert(config.id.as_str()) {
            return Err(AutomationError::Invalid {
                id: config.id.clone(),
                reason: "cet identifiant est déjà utilisé".into(),
            });
        }
        let automation = config.check()?;
        if config.enabled {
            automations.push(automation);
        }
    }
    Ok(automations)
}

impl AutomationConfig {
    /// Lit **une** automatisation écrite en YAML (une liste d'un seul
    /// élément, copiée depuis `automations.yaml`, est aussi acceptée).
    pub fn from_yaml(yaml: &str) -> Result<Self, AutomationError> {
        if yaml.len() > MAX_FILE_SIZE {
            return Err(AutomationError::TooLarge);
        }
        let yaml_error = |error: serde_saphyr::Error| AutomationError::Yaml(error.to_string());
        if yaml.trim_start().starts_with('-') {
            let mut list: Vec<Self> = serde_saphyr::from_str(yaml).map_err(yaml_error)?;
            return match (list.pop(), list.is_empty()) {
                (Some(config), true) => Ok(config),
                _ => Err(AutomationError::Yaml(
                    "une seule automatisation à la fois".into(),
                )),
            };
        }
        serde_saphyr::from_str(yaml).map_err(yaml_error)
    }

    /// Lit une automatisation envoyée en JSON par le tableau de bord.
    pub fn from_json(value: Value) -> Result<Self, AutomationError> {
        serde_json::from_value(value).map_err(|error| AutomationError::Yaml(error.to_string()))
    }

    /// La forme JSON, pour la base et le tableau de bord.
    #[must_use]
    pub fn to_json(&self) -> Value {
        // Ne peut pas échouer : toutes les clés sont des chaînes, et les
        // nombres sont finis (vérifiés à la lecture).
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// La forme YAML, pour l'affichage et l'édition.
    #[must_use]
    pub fn to_yaml(&self) -> String {
        serde_saphyr::to_string(self).unwrap_or_default()
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Le nom affiché (l'identifiant s'il n'y en a pas).
    #[must_use]
    pub fn alias(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.id)
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Vérifie chaque valeur et renvoie l'automatisation prête à l'emploi.
    pub fn check(&self) -> Result<Automation, AutomationError> {
        check(self.clone()).map_err(|reason| AutomationError::Invalid {
            id: self.id.clone(),
            reason,
        })
    }
}

/// Vérifie une automatisation. L'erreur est un message en français.
fn check(config: AutomationConfig) -> Result<Automation, String> {
    if !ha_core::is_valid_name(&config.id) {
        return Err("id : 1 à 64 caractères parmi a-z, 0-9 et « _ »".into());
    }
    let alias = config.alias.unwrap_or_else(|| config.id.clone());
    if alias.chars().count() > 255 || alias.chars().any(char::is_control) {
        return Err("alias trop long ou avec des caractères de contrôle".into());
    }
    if config.triggers.is_empty() || config.actions.is_empty() {
        return Err("il faut au moins un déclencheur (triggers) et une action (actions)".into());
    }
    for (name, len) in [
        ("triggers", config.triggers.len()),
        ("conditions", config.conditions.len()),
        ("actions", config.actions.len()),
    ] {
        if len > MAX_ITEMS {
            return Err(format!("{name} : {MAX_ITEMS} éléments au plus"));
        }
    }

    let max = match (config.mode, config.max) {
        (RunMode::Queued | RunMode::Parallel, Some(max)) if (1..=MAX_MAX_RUNS).contains(&max) => {
            max
        }
        (RunMode::Queued | RunMode::Parallel, Some(_)) => {
            return Err(format!("max : entre 1 et {MAX_MAX_RUNS}"));
        }
        (RunMode::Queued | RunMode::Parallel, None) => DEFAULT_MAX_RUNS,
        (_, Some(_)) => return Err("max ne sert qu'avec mode: queued ou parallel".into()),
        (_, None) => 1,
    };

    let mut triggers = Vec::with_capacity(config.triggers.len());
    let mut trigger_ids = Vec::with_capacity(config.triggers.len());
    for (index, trigger) in config.triggers.into_iter().enumerate() {
        let (trigger, id) = check_trigger(trigger)?;
        trigger_ids.push(match id {
            Some(id) => check_label(&id, "id de déclencheur")?,
            None => index.to_string(),
        });
        triggers.push(trigger);
    }
    let mut budget = Budget::default();
    let conditions = config
        .conditions
        .into_iter()
        .map(|c| check_condition(c, 0, &mut budget))
        .collect::<Result<_, _>>()?;
    let actions = config
        .actions
        .into_iter()
        .map(|a| check_action_in(a, 0, &mut budget))
        .collect::<Result<_, _>>()?;
    Ok(Automation {
        id: config.id,
        alias,
        triggers,
        trigger_ids,
        conditions,
        actions,
        mode: config.mode,
        max: usize::from(max),
    })
}

/// Profondeur maximale des blocs imbriqués (`if` dans `choose` dans…), et
/// nombre maximal d'actions et de conditions dans une automatisation : un
/// fichier ne peut pas construire une automatisation démesurée.
const MAX_DEPTH: usize = 5;
const MAX_NODES: usize = 200;

#[derive(Default)]
struct Budget {
    nodes: usize,
}

impl Budget {
    fn take(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("blocs imbriqués : {MAX_DEPTH} niveaux au plus"));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!("{MAX_NODES} actions et conditions au plus"));
        }
        Ok(())
    }
}

/// Un identifiant libre (id de déclencheur) : court et sans caractère de contrôle.
fn check_label(value: &str, what: &str) -> Result<String, String> {
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        return Err(format!("{what} {value:?} : 1 à 64 caractères"));
    }
    Ok(value.to_owned())
}

fn check_trigger(config: TriggerConfig) -> Result<(Trigger, Option<String>), String> {
    let id = match &config {
        TriggerConfig::State { id, .. }
        | TriggerConfig::NumericState { id, .. }
        | TriggerConfig::Time { id, .. }
        | TriggerConfig::Sun { id, .. }
        | TriggerConfig::TimePattern { id, .. }
        | TriggerConfig::Event { id, .. }
        | TriggerConfig::Calendar { id, .. } => id.clone(),
    };
    let trigger = match config {
        TriggerConfig::State {
            entity_id,
            attribute,
            from,
            to,
            not_from,
            not_to,
            for_,
            ..
        } => Trigger::State {
            entity_ids: entity_ids(entity_id)?,
            attribute: attribute
                .map(|name| check_label(&name, "attribute"))
                .transpose()?,
            from: values(from, "from")?,
            to: values(to, "to")?,
            not_from: values(not_from, "not_from")?,
            not_to: values(not_to, "not_to")?,
            for_: for_.as_deref().map(parse_duration).transpose()?,
        },
        TriggerConfig::NumericState {
            entity_id,
            above,
            below,
            ..
        } => {
            check_range(above, below)?;
            Trigger::NumericState {
                entity_ids: entity_ids(entity_id)?,
                above,
                below,
            }
        }
        TriggerConfig::Time { at, .. } => Trigger::Time {
            at: at
                .into_vec()
                .iter()
                .map(|t| parse_time_at(t))
                .collect::<Result<_, _>>()?,
        },
        TriggerConfig::Sun { event, offset, .. } => Trigger::Sun {
            event: parse_sun_event(&event)?,
            offset: offset
                .as_deref()
                .map(parse_offset)
                .transpose()?
                .unwrap_or_default(),
        },
        TriggerConfig::TimePattern {
            hours,
            minutes,
            seconds,
            ..
        } => check_time_pattern(hours, minutes, seconds)?,
        TriggerConfig::Calendar {
            entity_id,
            event,
            offset,
            ..
        } => {
            let entity_id = EntityId::new(&entity_id).map_err(|error| error.to_string())?;
            if entity_id.domain() != "calendar" {
                return Err(format!("calendar : {entity_id} n'est pas un agenda"));
            }
            let edge = match event.as_str() {
                "start" => CalendarEdge::Start,
                "end" => CalendarEdge::End,
                _ => return Err(format!("calendar : event {event:?} (start ou end)")),
            };
            let offset = offset
                .as_deref()
                .map(parse_offset)
                .transpose()?
                .unwrap_or_default();
            if offset.abs() > MAX_CALENDAR_OFFSET {
                return Err("calendar : le décalage est d'une semaine au plus".into());
            }
            Trigger::Calendar {
                entity_id,
                edge,
                offset,
            }
        }
        TriggerConfig::Event {
            event_type,
            event_data,
            ..
        } => {
            if !ha_core::is_valid_event_type(&event_type) {
                return Err(format!(
                    "event_type {event_type:?} : 1 à 64 caractères parmi a-z, 0-9, « _ » et « . »"
                ));
            }
            if event_data.len() > MAX_ITEMS {
                return Err(format!("event_data : {MAX_ITEMS} valeurs au plus"));
            }
            Trigger::Event {
                event_type,
                event_data,
            }
        }
    };
    Ok((trigger, id))
}

/// Un motif horaire, comme dans Home Assistant : les morceaux non indiqués
/// **plus petits** que le plus petit indiqué valent 0, les **plus grands**
/// valent `*`. `minutes: "/15"` : toutes les heures, aux minutes 0, 15, 30
/// et 45, à la seconde 0.
fn check_time_pattern(
    hours: Option<Text>,
    minutes: Option<Text>,
    seconds: Option<Text>,
) -> Result<Trigger, String> {
    let given = [hours.is_some(), minutes.is_some(), seconds.is_some()];
    let Some(smallest) = given.iter().rposition(|&g| g) else {
        return Err("time_pattern : indique hours, minutes ou seconds".into());
    };
    let part = |value: Option<Text>, position: usize, max: i8, name: &str| match value {
        Some(text) => parse_pattern(&text.0, max).ok_or_else(|| {
            format!(
                "time_pattern : {name} {:?} invalide (exemples : 7, \"/15\", \"*\")",
                text.0
            )
        }),
        None if position > smallest => Ok(Pattern::At(0)),
        None => Ok(Pattern::Any),
    };
    let hours = part(hours, 0, 23, "hours")?;
    let minutes = part(minutes, 1, 59, "minutes")?;
    // Au plus une fois par minute : la protection contre les boucles arrête
    // une automatisation lancée plus de 10 fois par minute.
    let Pattern::At(second) = part(seconds, 2, 59, "seconds")? else {
        return Err(
            "time_pattern : seconds doit être une valeur fixe (0 à 59) : \
                    une automatisation se lance au plus une fois par minute"
                .into(),
        );
    };
    Ok(Trigger::TimePattern {
        hours,
        minutes,
        second,
    })
}

fn parse_pattern(text: &str, max: i8) -> Option<Pattern> {
    let text = text.trim();
    if text == "*" {
        return Some(Pattern::Any);
    }
    let (every, number) = match text.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if number.is_empty() || number.len() > 2 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: i8 = number.parse().ok()?;
    match every {
        true if (1..=max).contains(&value) => Some(Pattern::Every(value)),
        false if (0..=max).contains(&value) => Some(Pattern::At(value)),
        _ => None,
    }
}

fn parse_sun_event(value: &str) -> Result<SunEvent, String> {
    SunEvent::parse(value).ok_or_else(|| {
        format!("événement du soleil {value:?} inconnu (sunrise, sunset, dawn ou dusk)")
    })
}

/// Décalage signé : `-00:30:00` (30 min avant), `01:00:00`, `+15min`…
fn parse_offset(value: &str) -> Result<SignedDuration, String> {
    let (negative, rest) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let duration = SignedDuration::try_from(parse_duration(rest)?)
        .map_err(|_| format!("décalage {value:?} invalide"))?;
    Ok(if negative { -duration } else { duration })
}

impl Trigger {
    /// La raison, en français : « Porte d'entrée passe à « on » », « il est
    /// 07:30 », « coucher du soleil »… Les noms des appareils viennent de
    /// leur `friendly_name`.
    #[must_use]
    pub fn describe(&self, states: &ha_core::StateMachine) -> String {
        let names = |ids: &[EntityId]| friendly_names(states, ids, " ou ");
        match self {
            Self::State {
                entity_ids,
                attribute,
                to,
                ..
            } => {
                let subject = match attribute {
                    Some(attribute) => format!("{attribute} de {}", names(entity_ids)),
                    None => names(entity_ids),
                };
                if to.is_empty() {
                    format!("{subject} change")
                } else {
                    format!("{subject} passe à {}", quoted(to))
                }
            }
            Self::NumericState { entity_ids, .. } => {
                format!("{} passe un seuil", names(entity_ids))
            }
            Self::Time { at } => {
                let times: Vec<String> = at
                    .iter()
                    .map(|t| match t {
                        TimeAt::Fixed(time) => time.strftime("%H:%M").to_string(),
                        TimeAt::Entity(id) => {
                            format!("l'heure de {}", names(std::slice::from_ref(id)))
                        }
                    })
                    .collect();
                format!("il est {}", times.join(" ou "))
            }
            Self::Sun { event, .. } => match event {
                SunEvent::Sunrise => "lever du soleil",
                SunEvent::Sunset => "coucher du soleil",
                SunEvent::Dawn => "aube",
                SunEvent::Dusk => "crépuscule",
            }
            .to_owned(),
            Self::Event { event_type, .. } => format!("événement {event_type}"),
            Self::Calendar {
                entity_id, edge, ..
            } => match edge {
                CalendarEdge::Start => format!(
                    "un rendez-vous de {} commence",
                    names(std::slice::from_ref(entity_id))
                ),
                CalendarEdge::End => format!(
                    "un rendez-vous de {} se termine",
                    names(std::slice::from_ref(entity_id))
                ),
            },
            Self::TimePattern { hours, minutes, .. } => match (hours, minutes) {
                (Pattern::Any, Pattern::Every(step)) => format!("toutes les {step} minutes"),
                (Pattern::Every(step), Pattern::At(0)) => format!("toutes les {step} heures"),
                (Pattern::Any, Pattern::At(0)) => "toutes les heures".to_owned(),
                _ => "motif horaire".to_owned(),
            },
        }
    }
}

/// Les noms des entités (leur `friendly_name`, sinon leur identifiant),
/// séparés par `separator`.
fn friendly_names(states: &ha_core::StateMachine, ids: &[EntityId], separator: &str) -> String {
    ids.iter()
        .map(|id| {
            states
                .get(id)
                .and_then(|s| {
                    s.attributes()
                        .get("friendly_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| id.as_str().to_owned())
        })
        .collect::<Vec<_>>()
        .join(separator)
}

fn sun_name(event: SunEvent) -> &'static str {
    match event {
        SunEvent::Sunrise => "le lever du soleil",
        SunEvent::Sunset => "le coucher du soleil",
        SunEvent::Dawn => "l'aube",
        SunEvent::Dusk => "le crépuscule",
    }
}

/// « 5 min », « 1 h 30 min », « 45 s ».
fn duration_text(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours} h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes} min"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds} s"));
    }
    parts.join(" ")
}

/// « « Orange » ou « Rouge » ».
fn quoted(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("« {value} »"))
        .collect::<Vec<_>>()
        .join(" ou ")
}

/// Un nombre sans « .0 » inutile : `20` et non `20.0`.
fn number_text(value: f64) -> String {
    let text = value.to_string();
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
}

impl Condition {
    /// La condition, en français : « Porte d'entrée est « off » », « après
    /// 07:00 »… (pour l'historique des exécutions).
    #[must_use]
    pub fn describe(&self, states: &ha_core::StateMachine) -> String {
        match self {
            Self::State {
                entity_ids,
                attribute,
                states: expected,
            } => {
                let names = friendly_names(states, entity_ids, " et ");
                let subject = match attribute {
                    Some(attribute) => format!("{attribute} de {names}"),
                    None => names,
                };
                format!("{subject} est {}", quoted(expected))
            }
            Self::NumericState {
                entity_ids,
                above,
                below,
            } => {
                let names = friendly_names(states, entity_ids, " et ");
                match (above, below) {
                    (Some(above), Some(below)) => format!(
                        "{names} est entre {} et {}",
                        number_text(*above),
                        number_text(*below)
                    ),
                    (Some(above), None) => {
                        format!("{names} est au-dessus de {}", number_text(*above))
                    }
                    (None, Some(below)) => {
                        format!("{names} est en dessous de {}", number_text(*below))
                    }
                    (None, None) => format!("{names} est un nombre"),
                }
            }
            Self::Time {
                after,
                before,
                weekdays,
            } => {
                let mut parts = Vec::new();
                if let Some(after) = after {
                    parts.push(format!("après {}", after.strftime("%H:%M")));
                }
                if let Some(before) = before {
                    parts.push(format!("avant {}", before.strftime("%H:%M")));
                }
                if !weekdays.is_empty() {
                    let days: Vec<&str> = weekdays.iter().map(|day| weekday_name(*day)).collect();
                    parts.push(format!("le {}", days.join(", ")));
                }
                if parts.is_empty() {
                    "à toute heure".to_owned()
                } else {
                    parts.join(", ")
                }
            }
            Self::Sun { after, before } => {
                let mut parts = Vec::new();
                if let Some((event, _)) = after {
                    parts.push(format!("après {}", sun_name(*event)));
                }
                if let Some((event, _)) = before {
                    parts.push(format!("avant {}", sun_name(*event)));
                }
                parts.join(", ")
            }
            Self::And(list) => format!("toutes ces conditions ({})", list.len()),
            Self::Or(list) => format!("l'une de ces conditions ({})", list.len()),
            Self::Not(list) => format!("aucune de ces conditions ({})", list.len()),
            Self::Trigger(ids) => {
                let ids: Vec<String> = ids.iter().map(|id| format!("« {id} »")).collect();
                format!("lancée par le déclencheur {}", ids.join(" ou "))
            }
            Self::Template { source, .. } => format!("le modèle {} est vrai", source.trim()),
        }
    }
}

fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "lundi",
        Weekday::Tuesday => "mardi",
        Weekday::Wednesday => "mercredi",
        Weekday::Thursday => "jeudi",
        Weekday::Friday => "vendredi",
        Weekday::Saturday => "samedi",
        Weekday::Sunday => "dimanche",
    }
}

impl Action {
    /// L'action, en français (pour l'historique des exécutions). Les données
    /// envoyées (`data`) ne sont **pas** reprises : elles peuvent contenir un
    /// code d'alarme.
    #[must_use]
    pub fn describe(&self, states: &ha_core::StateMachine) -> String {
        match self {
            Self::Service {
                domain,
                service,
                entity_ids,
                ..
            } => format!(
                "{domain}.{service} sur {}",
                friendly_names(states, entity_ids, ", ")
            ),
            Self::Delay(duration) => format!("attendre {}", duration_text(*duration)),
            Self::If { .. } => "si… alors… sinon".to_owned(),
            Self::Choose { options, .. } => format!("choisir parmi {} option(s)", options.len()),
            Self::Condition(condition) => {
                format!("continuer si : {}", condition.describe(states))
            }
            Self::Repeat { until, .. } => match until {
                RepeatUntil::Count(count) => format!("répéter {count} fois"),
                RepeatUntil::While(_) => "répéter tant que…".to_owned(),
                RepeatUntil::Until(_) => "répéter jusqu'à ce que…".to_owned(),
            },
            Self::WaitForTrigger { timeout, .. } => match timeout {
                Some(timeout) => format!(
                    "attendre un déclencheur (au plus {})",
                    duration_text(*timeout)
                ),
                None => "attendre un déclencheur".to_owned(),
            },
        }
    }
}

/// Les automatisations utilisent-elles le soleil ? (Il faut alors la
/// position de la maison.)
#[must_use]
pub fn uses_sun(automations: &[Automation]) -> bool {
    automations.iter().any(|automation| {
        automation
            .triggers
            .iter()
            .any(|t| matches!(t, Trigger::Sun { .. }))
            || automation.conditions.iter().any(Condition::uses_sun)
            || automation.actions.iter().any(Action::uses_sun)
    })
}

impl Condition {
    fn uses_sun(&self) -> bool {
        match self {
            Self::Sun { .. } => true,
            Self::And(list) | Self::Or(list) | Self::Not(list) => list.iter().any(Self::uses_sun),
            _ => false,
        }
    }

    fn collect_entities<'a>(&'a self, ids: &mut Vec<&'a EntityId>) {
        match self {
            Self::State { entity_ids, .. } | Self::NumericState { entity_ids, .. } => {
                ids.extend(entity_ids);
            }
            Self::And(list) | Self::Or(list) | Self::Not(list) => {
                for condition in list {
                    condition.collect_entities(ids);
                }
            }
            Self::Template { template, .. } => ids.extend(template.entity_ids()),
            _ => {}
        }
    }
}

impl Action {
    fn uses_sun(&self) -> bool {
        match self {
            Self::Condition(condition) => condition.uses_sun(),
            Self::If {
                conditions,
                then,
                otherwise,
            } => {
                conditions.iter().any(Condition::uses_sun)
                    || then.iter().chain(otherwise).any(Self::uses_sun)
            }
            Self::Choose { options, default } => {
                options.iter().any(|(conditions, actions)| {
                    conditions.iter().any(Condition::uses_sun) || actions.iter().any(Self::uses_sun)
                }) || default.iter().any(Self::uses_sun)
            }
            Self::Repeat { until, actions } => {
                let conditions = match until {
                    RepeatUntil::Count(_) => &[][..],
                    RepeatUntil::While(list) | RepeatUntil::Until(list) => list,
                };
                conditions.iter().any(Condition::uses_sun) || actions.iter().any(Self::uses_sun)
            }
            Self::Service { .. } | Self::Delay(_) | Self::WaitForTrigger { .. } => false,
        }
    }

    fn collect_entities<'a>(&'a self, ids: &mut Vec<&'a EntityId>) {
        match self {
            Self::Service {
                entity_ids,
                templates,
                ..
            } => {
                ids.extend(entity_ids);
                ids.extend(templates.entity_ids());
            }
            Self::Delay(_) => {}
            Self::WaitForTrigger { triggers, .. } => {
                for trigger in triggers {
                    if let Trigger::State { entity_ids, .. }
                    | Trigger::NumericState { entity_ids, .. } = trigger
                    {
                        ids.extend(entity_ids);
                    }
                }
            }
            Self::Repeat { until, actions } => {
                if let RepeatUntil::While(list) | RepeatUntil::Until(list) = until {
                    for condition in list {
                        condition.collect_entities(ids);
                    }
                }
                for action in actions {
                    action.collect_entities(ids);
                }
            }
            Self::Condition(condition) => condition.collect_entities(ids),
            Self::If {
                conditions,
                then,
                otherwise,
            } => {
                for condition in conditions {
                    condition.collect_entities(ids);
                }
                for action in then.iter().chain(otherwise) {
                    action.collect_entities(ids);
                }
            }
            Self::Choose { options, default } => {
                for (conditions, actions) in options {
                    for condition in conditions {
                        condition.collect_entities(ids);
                    }
                    for action in actions {
                        action.collect_entities(ids);
                    }
                }
                for action in default {
                    action.collect_entities(ids);
                }
            }
        }
    }
}

impl Automation {
    /// Toutes les entités citées (déclencheurs, conditions, actions), triées.
    #[must_use]
    pub fn entity_ids(&self) -> Vec<&EntityId> {
        let mut ids: Vec<&EntityId> = Vec::new();
        for trigger in &self.triggers {
            if let Trigger::State { entity_ids, .. } | Trigger::NumericState { entity_ids, .. } =
                trigger
            {
                ids.extend(entity_ids);
            }
        }
        for condition in &self.conditions {
            condition.collect_entities(&mut ids);
        }
        for action in &self.actions {
            action.collect_entities(&mut ids);
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

fn check_condition(
    config: ConditionConfig,
    depth: usize,
    budget: &mut Budget,
) -> Result<Condition, String> {
    budget.take(depth)?;
    let mut nested = |list: Vec<ConditionConfig>, name: &str| -> Result<Vec<Condition>, String> {
        if list.is_empty() {
            return Err(format!("condition {name} : il faut au moins une condition"));
        }
        list.into_iter()
            .map(|c| check_condition(c, depth + 1, budget))
            .collect()
    };
    Ok(match config {
        ConditionConfig::And { conditions } => Condition::And(nested(conditions, "and")?),
        ConditionConfig::Or { conditions } => Condition::Or(nested(conditions, "or")?),
        ConditionConfig::Not { conditions } => Condition::Not(nested(conditions, "not")?),
        ConditionConfig::Template { value_template } => {
            let template = Template::parse(&value_template)
                .map_err(|error| format!("condition template : {error}"))?
                .ok_or("condition template : un modèle {{ … }} attendu dans value_template")?;
            Condition::Template {
                source: value_template,
                template,
            }
        }
        ConditionConfig::Trigger { id } => Condition::Trigger(
            id.into_vec()
                .iter()
                .map(|id| check_label(id, "id de déclencheur"))
                .collect::<Result<_, _>>()?,
        ),
        ConditionConfig::State {
            entity_id,
            attribute,
            state,
        } => Condition::State {
            entity_ids: entity_ids(entity_id)?,
            attribute: attribute
                .map(|name| check_label(&name, "attribute"))
                .transpose()?,
            states: values(Some(state), "state")?,
        },
        ConditionConfig::NumericState {
            entity_id,
            above,
            below,
        } => {
            check_range(above, below)?;
            Condition::NumericState {
                entity_ids: entity_ids(entity_id)?,
                above,
                below,
            }
        }
        ConditionConfig::Time {
            after,
            before,
            weekday,
        } => {
            if after.is_none() && before.is_none() && weekday.is_none() {
                return Err("condition time : il faut after, before ou weekday".into());
            }
            Condition::Time {
                after: after.as_deref().map(parse_time).transpose()?,
                before: before.as_deref().map(parse_time).transpose()?,
                weekdays: weekday
                    .map(OneOrMany::into_vec)
                    .unwrap_or_default()
                    .iter()
                    .map(|day| parse_weekday(day))
                    .collect::<Result<_, _>>()?,
            }
        }
        ConditionConfig::Sun {
            after,
            before,
            after_offset,
            before_offset,
        } => {
            if after.is_none() && before.is_none() {
                return Err("condition sun : il faut after ou before".into());
            }
            let with_offset = |event: Option<String>, offset: Option<String>| {
                if event.is_none() && offset.is_some() {
                    return Err("condition sun : décalage sans événement".to_owned());
                }
                event
                    .map(|event| {
                        Ok((
                            parse_sun_event(&event)?,
                            offset
                                .as_deref()
                                .map(parse_offset)
                                .transpose()?
                                .unwrap_or_default(),
                        ))
                    })
                    .transpose()
            };
            Condition::Sun {
                after: with_offset(after, after_offset)?,
                before: with_offset(before, before_offset)?,
            }
        }
    })
}

/// Vérifie une action (utilisée aussi par les scripts).
pub(crate) fn check_action(config: ActionConfig) -> Result<Action, String> {
    check_action_in(config, 0, &mut Budget::default())
}

fn check_action_in(
    config: ActionConfig,
    depth: usize,
    budget: &mut Budget,
) -> Result<Action, String> {
    budget.take(depth)?;
    let actions = |list: Vec<ActionConfig>, budget: &mut Budget| -> Result<Vec<Action>, String> {
        list.into_iter()
            .map(|a| check_action_in(a, depth + 1, budget))
            .collect()
    };
    let conditions =
        |list: Vec<ConditionConfig>, budget: &mut Budget| -> Result<Vec<Condition>, String> {
            if list.is_empty() {
                return Err("il faut au moins une condition".into());
            }
            list.into_iter()
                .map(|c| check_condition(c, depth + 1, budget))
                .collect()
        };
    match config {
        ActionConfig::Condition(condition) => Ok(Action::Condition(check_condition(
            condition,
            depth + 1,
            budget,
        )?)),
        ActionConfig::If(config) => {
            if config.then.is_empty() {
                return Err("if : il faut au moins une action dans then".into());
            }
            Ok(Action::If {
                conditions: conditions(config.if_, budget)?,
                then: actions(config.then, budget)?,
                otherwise: actions(config.else_.unwrap_or_default(), budget)?,
            })
        }
        ActionConfig::Choose(config) => {
            if config.choose.is_empty() {
                return Err("choose : il faut au moins une option".into());
            }
            let mut options = Vec::with_capacity(config.choose.len());
            for option in config.choose {
                if option.sequence.is_empty() {
                    return Err("choose : chaque option doit avoir une sequence".into());
                }
                options.push((
                    conditions(option.conditions, budget)?,
                    actions(option.sequence, budget)?,
                ));
            }
            Ok(Action::Choose {
                options,
                default: actions(config.default.unwrap_or_default(), budget)?,
            })
        }
        ActionConfig::Repeat(RepeatActionConfig { repeat }) => {
            if repeat.sequence.is_empty() {
                return Err("repeat : il faut au moins une action dans sequence".into());
            }
            let until = match (repeat.count, repeat.while_, repeat.until) {
                (Some(count), None, None) if (1..=MAX_REPEAT).contains(&count) => {
                    RepeatUntil::Count(count)
                }
                (Some(_), None, None) => {
                    return Err(format!("repeat : count entre 1 et {MAX_REPEAT}"));
                }
                (None, Some(list), None) => RepeatUntil::While(conditions(list, budget)?),
                (None, None, Some(list)) => RepeatUntil::Until(conditions(list, budget)?),
                _ => return Err("repeat : il faut count, while ou until (un seul)".into()),
            };
            Ok(Action::Repeat {
                until,
                actions: actions(repeat.sequence, budget)?,
            })
        }
        ActionConfig::WaitForTrigger(wait) => {
            if wait.wait_for_trigger.is_empty() || wait.wait_for_trigger.len() > MAX_ITEMS {
                return Err(format!("wait_for_trigger : 1 à {MAX_ITEMS} déclencheurs"));
            }
            let mut triggers = Vec::with_capacity(wait.wait_for_trigger.len());
            for config in wait.wait_for_trigger {
                budget.take(depth + 1)?;
                let (trigger, _) = check_trigger(config)?;
                if matches!(trigger, Trigger::Sun { .. }) {
                    return Err("wait_for_trigger : le soleil n'est pas possible ici \
                         (utilise une condition sun)"
                        .into());
                }
                triggers.push(trigger);
            }
            Ok(Action::WaitForTrigger {
                triggers,
                timeout: wait.timeout.as_deref().map(parse_duration).transpose()?,
                continue_on_timeout: wait.continue_on_timeout.unwrap_or(true),
            })
        }
        ActionConfig::Delay(delay) => Ok(Action::Delay(parse_duration(&delay.delay)?)),
        ActionConfig::Service(service) => {
            let Some((domain, name)) = service.action.split_once('.') else {
                return Err(format!(
                    "action {:?} : format attendu domaine.service, par exemple light.turn_on",
                    service.action
                ));
            };
            for part in [domain, name] {
                if !ha_core::is_valid_name(part) {
                    return Err(format!("action {:?} invalide", service.action));
                }
            }
            let Some(target) = service.target else {
                return Err(format!(
                    "action {} : il faut une cible (target: entity_id: …)",
                    service.action
                ));
            };
            let templates = DataTemplates::find(&service.data)?;
            Ok(Action::Service {
                domain: domain.to_owned(),
                service: name.to_owned(),
                entity_ids: entity_ids(target.entity_id)?,
                data: service.data,
                templates,
            })
        }
    }
}

/// Une ou plusieurs valeurs (`to: Orange` ou `to: [Orange, Rouge]`).
fn values(value: Option<OneOrMany>, name: &str) -> Result<Vec<String>, String> {
    let list = value.map(OneOrMany::into_vec).unwrap_or_default();
    if list.len() > MAX_ITEMS {
        return Err(format!("{name} : {MAX_ITEMS} valeurs au plus"));
    }
    if list.iter().any(|v| v.chars().count() > 255) {
        return Err(format!("{name} : valeur trop longue"));
    }
    Ok(list)
}

/// Où se trouve un modèle dans `data` : `message`, `data › 0 › titre`…
#[derive(Debug, Clone, PartialEq)]
enum DataKey {
    Field(String),
    Index(usize),
}

/// Les modèles d'une action (`{{ … }}` dans `data`), et leur place.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataTemplates(Vec<(Vec<DataKey>, Template)>);

/// Profondeur maximale explorée dans `data`, et nombre de modèles au plus.
const MAX_DATA_DEPTH: usize = 4;
const MAX_TEMPLATES: usize = 10;

impl DataTemplates {
    /// Trouve et vérifie les modèles dans `data`.
    fn find(data: &Attributes) -> Result<Self, String> {
        let mut found = Vec::new();
        for (key, value) in data {
            find_templates(value, &mut vec![DataKey::Field(key.clone())], &mut found)?;
        }
        if found.len() > MAX_TEMPLATES {
            return Err(format!("data : {MAX_TEMPLATES} modèles au plus"));
        }
        Ok(Self(found))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Les données avec les modèles calculés.
    pub(crate) fn render(&self, data: &Attributes, scope: &Scope<'_>) -> Attributes {
        let mut data = data.clone();
        for (path, template) in &self.0 {
            let Some((DataKey::Field(first), rest)) = path.split_first() else {
                continue;
            };
            if let Some(slot) = data.get_mut(first).and_then(|value| slot_at(value, rest)) {
                *slot = template.render(scope);
            }
        }
        data
    }

    fn entity_ids(&self) -> impl Iterator<Item = &EntityId> {
        self.0
            .iter()
            .flat_map(|(_, template)| template.entity_ids())
    }
}

/// La valeur à la place `path` dans `value`.
fn slot_at<'v>(value: &'v mut Value, path: &[DataKey]) -> Option<&'v mut Value> {
    let Some((key, rest)) = path.split_first() else {
        return Some(value);
    };
    let next = match (key, value) {
        (DataKey::Field(name), Value::Object(map)) => map.get_mut(name),
        (DataKey::Index(index), Value::Array(list)) => list.get_mut(*index),
        _ => None,
    };
    slot_at(next?, rest)
}

fn find_templates(
    value: &Value,
    path: &mut Vec<DataKey>,
    found: &mut Vec<(Vec<DataKey>, Template)>,
) -> Result<(), String> {
    if path.len() > MAX_DATA_DEPTH {
        return Ok(());
    }
    match value {
        Value::String(text) => {
            if let Some(template) = Template::parse(text).map_err(|error| {
                let place: Vec<String> = path
                    .iter()
                    .map(|key| match key {
                        DataKey::Field(name) => name.clone(),
                        DataKey::Index(index) => index.to_string(),
                    })
                    .collect();
                format!("data.{} : {error}", place.join("."))
            })? {
                found.push((path.clone(), template));
            }
        }
        Value::Array(list) => {
            for (index, item) in list.iter().enumerate() {
                path.push(DataKey::Index(index));
                find_templates(item, path, found)?;
                path.pop();
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                path.push(DataKey::Field(key.clone()));
                find_templates(item, path, found)?;
                path.pop();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn entity_ids(value: OneOrMany) -> Result<Vec<EntityId>, String> {
    let ids = value.into_vec();
    if ids.is_empty() || ids.len() > MAX_ITEMS {
        return Err(format!("entity_id : 1 à {MAX_ITEMS} entités"));
    }
    ids.iter()
        .map(|id| EntityId::new(id).map_err(|error| error.to_string()))
        .collect()
}

fn check_range(above: Option<f64>, below: Option<f64>) -> Result<(), String> {
    match (above, below) {
        (None, None) => Err("numeric_state : il faut above ou below".into()),
        (Some(a), Some(b)) if a >= b => {
            Err("numeric_state : above doit être inférieur à below".into())
        }
        (a, b) if a.is_some_and(|v| !v.is_finite()) || b.is_some_and(|v| !v.is_finite()) => {
            Err("numeric_state : nombre invalide".into())
        }
        _ => Ok(()),
    }
}

/// `"07:30"` ou `"07:30:15"`.
/// `07:30`, ou une entrée `input_datetime.reveil`.
fn parse_time_at(value: &str) -> Result<TimeAt, String> {
    if !value.contains('.') {
        return parse_time(value).map(TimeAt::Fixed);
    }
    let entity_id = EntityId::new(value).map_err(|error| error.to_string())?;
    if entity_id.domain() != "input_datetime" {
        return Err(format!(
            "time : {entity_id} n'est ni une heure (07:30) ni une entrée input_datetime"
        ));
    }
    Ok(TimeAt::Entity(entity_id))
}

fn parse_time(value: &str) -> Result<Time, String> {
    let parts: Vec<&str> = value.split(':').collect();
    let numbers: Option<Vec<i8>> = parts.iter().map(|p| p.parse().ok()).collect();
    let time = match (parts.len(), numbers) {
        (2, Some(n)) => Time::new(n[0], n[1], 0, 0),
        (3, Some(n)) => Time::new(n[0], n[1], n[2], 0),
        _ => {
            return Err(format!(
                "heure {value:?} invalide (format HH:MM ou HH:MM:SS)"
            ));
        }
    };
    time.map_err(|_| format!("heure {value:?} invalide"))
}

/// `"00:05:00"` (heures:minutes:secondes) ou `"30s"`, `"5min"`, `"2h"`.
fn parse_duration(value: &str) -> Result<Duration, String> {
    let invalid = || format!("durée {value:?} invalide (exemples : 00:05:00, 30s, 5min, 2h)");
    let duration = if value.contains(':') {
        let time = parse_time(value).map_err(|_| invalid())?;
        Duration::from_secs(
            u64::try_from(time.hour()).unwrap_or(0) * 3600
                + u64::try_from(time.minute()).unwrap_or(0) * 60
                + u64::try_from(time.second()).unwrap_or(0),
        )
    } else {
        let split = value
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(invalid)?;
        let amount: u64 = value[..split].parse().map_err(|_| invalid())?;
        let seconds = match &value[split..] {
            "s" => amount,
            "min" => amount.saturating_mul(60),
            "h" => amount.saturating_mul(3600),
            _ => return Err(invalid()),
        };
        Duration::from_secs(seconds)
    };
    if duration > MAX_DURATION {
        return Err(format!("durée {value:?} trop longue (24 h au plus)"));
    }
    Ok(duration)
}

/// Jours en anglais (comme Home Assistant) ou en français.
fn parse_weekday(value: &str) -> Result<Weekday, String> {
    Ok(match value {
        "mon" | "lun" => Weekday::Monday,
        "tue" | "mar" => Weekday::Tuesday,
        "wed" | "mer" => Weekday::Wednesday,
        "thu" | "jeu" => Weekday::Thursday,
        "fri" | "ven" => Weekday::Friday,
        "sat" | "sam" => Weekday::Saturday,
        "sun" | "dim" => Weekday::Sunday,
        _ => return Err(format!("jour {value:?} invalide (mon…sun ou lun…dim)")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
- id: salon_au_retour
  alias: Allumer le salon quand on rentre le soir
  triggers:
    - trigger: state
      entity_id: binary_sensor.porte_entree
      to: "on"
  conditions:
    - condition: time
      after: "18:00"
      before: "23:30"
    - condition: state
      entity_id: light.salon
      state: "off"
  actions:
    - action: light.turn_on
      target:
        entity_id: light.salon
      data:
        brightness: 180
    - delay: 10min
    - action: light.turn_off
      target:
        entity_id: [light.salon, light.couloir]

- id: chauffage
  triggers:
    - trigger: numeric_state
      entity_id: sensor.temperature_salon
      below: 18.5
    - trigger: time
      at: ["06:30", "18:00:00"]
  conditions:
    - condition: time
      weekday: [lun, mar, wed]
  actions:
    - service: switch.turn_on
      target:
        entity_id: switch.radiateur

- id: desactivee
  enabled: false
  triggers: [{trigger: time, at: "12:00"}]
  actions: [{delay: 1s}]
"#;

    #[test]
    fn example_file_is_understood() {
        let automations = parse(EXAMPLE).unwrap();
        assert_eq!(
            automations.len(),
            2,
            "l'automatisation désactivée est ignorée"
        );
        let first = &automations[0];
        assert_eq!(first.alias, "Allumer le salon quand on rentre le soir");
        assert_eq!(first.actions[1], Action::Delay(Duration::from_mins(10)));
        let Action::Service { entity_ids, .. } = &first.actions[2] else {
            panic!("service attendu");
        };
        assert_eq!(entity_ids.len(), 2);
        let second = &automations[1];
        assert_eq!(second.alias, "chauffage");
        let Trigger::Time { at } = &second.triggers[1] else {
            panic!("time attendu");
        };
        assert_eq!(at[0], TimeAt::Fixed(Time::new(6, 30, 0, 0).unwrap()));
    }

    #[test]
    fn mistakes_are_reported() {
        let cases = [
            // Faute de frappe dans un champ.
            "- id: a\n  triggers: [{trigger: state, entity_id: light.a, too: 'on'}]\n  actions: [{delay: 1s}]",
            // Déclencheur inconnu.
            "- id: a\n  triggers: [{trigger: moon, event: moonrise}]\n  actions: [{delay: 1s}]",
            // Identifiant d'entité invalide.
            "- id: a\n  triggers: [{trigger: state, entity_id: Salon}]\n  actions: [{delay: 1s}]",
            // Heure invalide.
            "- id: a\n  triggers: [{trigger: time, at: '25:00'}]\n  actions: [{delay: 1s}]",
            // Durée trop longue.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{delay: 48h}]",
            // Action sans cible.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{action: light.turn_on}]",
            // Action mal écrite.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{action: allume, target: {entity_id: light.a}}]",
            // Pas d'action.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: []",
            // Identifiant d'automatisation invalide.
            "- id: Mon Automatisation\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{delay: 1s}]",
            // Intervalle impossible.
            "- id: a\n  triggers: [{trigger: numeric_state, entity_id: sensor.t, above: 20, below: 10}]\n  actions: [{delay: 1s}]",
            // Doublon d'identifiant.
            "- id: a\n  triggers: [{trigger: time, at: '12:00'}]\n  actions: [{delay: 1s}]\n- id: a\n  triggers: [{trigger: time, at: '13:00'}]\n  actions: [{delay: 1s}]",
            // Pas du YAML attendu.
            "ceci n'est pas une liste",
        ];
        for yaml in cases {
            assert!(parse(yaml).is_err(), "aurait dû être refusé :\n{yaml}");
        }
    }

    #[test]
    fn yaml_bombs_are_refused() {
        // « Billion laughs » : des alias qui se multiplient.
        let bomb = "a: &a [x,x,x,x,x,x,x,x,x,x]\nb: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a,*a]\nc: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b,*b]\nd: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c,*c]\ne: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d,*d]\nf: &f [*e,*e,*e,*e,*e,*e,*e,*e,*e,*e]\ng: [*f,*f,*f,*f,*f,*f,*f,*f,*f,*f]\n";
        // Même lu comme une valeur quelconque, il est refusé par la bibliothèque.
        let generic: Result<serde_json::Value, _> = serde_saphyr::from_str(bomb);
        let error = generic.unwrap_err().to_string();
        assert!(
            error.contains("budget") || error.contains("alias"),
            "{error}"
        );
        assert!(parse(bomb).is_err());
        assert!(matches!(
            parse(&"#".repeat(MAX_FILE_SIZE + 1)),
            Err(AutomationError::TooLarge)
        ));
    }

    #[test]
    fn durations_and_times() {
        assert_eq!(parse_duration("00:05:00").unwrap(), Duration::from_mins(5));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_hours(2));
        for bad in ["", "5", "5 min", "-1s", "1d", "99:00:00"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
        assert!(parse_time("7:05").is_ok());
        for bad in ["7", "24:00", "07:60", "a:b"] {
            assert!(parse_time(bad).is_err(), "{bad:?}");
        }
    }
}
