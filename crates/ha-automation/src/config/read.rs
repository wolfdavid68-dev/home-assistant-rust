//! Ce qu'on lit : le YAML (ou le JSON du tableau de bord) transformé en
//! structures Rust (`*Config`), avant toute vérification.
//!
//! Les champs ne sont visibles que du module `config` (`pub(super)`) : la
//! vérification, rangée à côté, les lit.

use ha_core::Attributes;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
pub(crate) struct Text(pub(super) String);

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
    pub(super) fn into_vec(self) -> Vec<String> {
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
    pub(super) id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) alias: Option<String>,
    #[serde(default = "enabled_by_default", skip_serializing_if = "is_true")]
    pub(super) enabled: bool,
    /// Que faire si elle se redéclenche pendant qu'elle tourne encore.
    #[serde(default, skip_serializing_if = "RunMode::is_single")]
    pub(super) mode: RunMode,
    /// Nombre maximal d'exécutions en attente ou simultanées (`queued`, `parallel`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) max: Option<u8>,
    pub(super) triggers: Vec<TriggerConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) conditions: Vec<ConditionConfig>,
    pub(super) actions: Vec<ActionConfig>,
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
pub(super) const DEFAULT_MAX_RUNS: u8 = 10;
pub(super) const MAX_MAX_RUNS: u8 = 20;

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
pub(super) enum TriggerConfig {
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
    pub(super) repeat: RepeatConfig,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepeatConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) count: Option<u32>,
    #[serde(default, rename = "while", skip_serializing_if = "Option::is_none")]
    pub(super) while_: Option<Vec<ConditionConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) until: Option<Vec<ConditionConfig>>,
    pub(super) sequence: Vec<ActionConfig>,
}

/// `wait_for_trigger:` (déclencheurs), `timeout:` et `continue_on_timeout:`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaitActionConfig {
    pub(super) wait_for_trigger: Vec<TriggerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) continue_on_timeout: Option<bool>,
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
    pub(super) if_: Vec<ConditionConfig>,
    pub(super) then: Vec<ActionConfig>,
    #[serde(default, rename = "else", skip_serializing_if = "Option::is_none")]
    pub(super) else_: Option<Vec<ActionConfig>>,
}

/// `choose:` une liste d'options (la première dont les conditions sont
/// vraies est exécutée), et `default:` si aucune ne l'est.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChooseActionConfig {
    pub(super) choose: Vec<ChooseOptionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) default: Option<Vec<ActionConfig>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChooseOptionConfig {
    pub(super) conditions: Vec<ConditionConfig>,
    pub(super) sequence: Vec<ActionConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceActionConfig {
    /// `light.turn_on` (Home Assistant accepte aussi l'ancien nom `service`).
    #[serde(alias = "service")]
    pub(super) action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) target: Option<TargetConfig>,
    #[serde(default, skip_serializing_if = "Attributes::is_empty")]
    pub(super) data: Attributes,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetConfig {
    pub(super) entity_id: OneOrMany,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DelayActionConfig {
    pub(super) delay: String,
}
