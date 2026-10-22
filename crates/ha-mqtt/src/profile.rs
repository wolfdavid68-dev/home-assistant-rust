//! Ce qu'un type d'appareil sait faire au-delà de « marche / arrêt » :
//! verrou, vanne, aspirateur, volet, ventilateur, thermostat, alarme…
//!
//! Un [`Profile`] décrit, pour une entité :
//!
//! - les **états** possibles (« LOCKED » reçu → `locked`) ;
//! - les **sujets d'attributs** (consigne d'humidité, batterie…) ;
//! - ce qu'il faut **publier** pour chaque service (`lock.unlock` → `UNLOCK`) ;
//! - des **attributs fixes** (modes possibles, bornes) utiles à l'interface.
//!
//! Sécurité : un service n'envoie jamais un texte venu de l'appelant tel quel.
//! Un nombre est vérifié (fini, entre les bornes annoncées), un choix doit
//! faire partie de la liste annoncée par l'appareil.

use ha_core::Attributes;
use serde_json::{Value, json};

use crate::template::Template;

/// Ce qu'on attend sur un sujet d'attribut.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrKind {
    /// Un nombre : autre chose est ignoré.
    Number,
    /// Un texte court, sans caractère de contrôle.
    Text,
    /// Oui / non : `on` donne `true`, `off` donne `false` (l'oscillation d'un ventilateur).
    Flag { on: String, off: String },
    /// Un nombre de l'appareil, ramené de `[min, max]` à un pourcentage
    /// (position d'un volet de 0 à 255, vitesse d'un ventilateur de 1 à 6…).
    Percent { min: f64, max: f64 },
}

/// Un sujet qui donne un attribut de l'entité (`humidity`, `battery_level`…).
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeTopic {
    pub name: &'static str,
    pub topic: String,
    pub template: Template,
    pub kind: AttrKind,
}

/// L'état d'une entité qui n'est ni « on / off », ni une valeur de liste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FreeState {
    /// Seulement les états de la liste [`Profile::states`].
    #[default]
    Listed,
    /// Un nombre (entité `number`).
    Number,
    /// Un texte de `max` caractères au plus (entité `text`).
    Text { max: usize },
}

/// Sans sujet d'état, ce qu'on suppose après une commande (mode « optimiste »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Optimistic {
    /// Rien : on attend que l'appareil le dise.
    Keep,
    /// Cet état (`locked` après `lock`).
    State(&'static str),
    /// La valeur envoyée (le choix d'une liste, le nombre réglé).
    Sent,
    /// L'heure de la commande (un bouton : la date du dernier appui).
    Now,
}

/// Ce qu'on publie pour un service.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// Toujours le même texte (`LOCK`, `start`…).
    Fixed(String),
    /// Un nombre lu dans les paramètres du service, entre deux bornes.
    /// Avec `scale`, il est ramené à l'échelle de l'appareil (un pourcentage
    /// 0–100 devient 0–255, par exemple) et arrondi.
    Number {
        field: &'static str,
        min: f64,
        max: f64,
        scale: Option<(f64, f64)>,
    },
    /// Oui / non lu dans les paramètres : `on` ou `off` est envoyé.
    Flag {
        field: &'static str,
        on: String,
        off: String,
    },
    /// Un texte lu dans les paramètres, de `min` à `max` caractères, sans
    /// caractère de contrôle.
    Text {
        field: &'static str,
        min: usize,
        max: usize,
    },
    /// Un choix lu dans les paramètres, parmi une liste annoncée.
    Choice {
        field: &'static str,
        options: Vec<String>,
    },
}

/// Un service et le message qu'il publie.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceCommand {
    pub service: &'static str,
    pub topic: String,
    pub payload: Payload,
    /// Sans sujet d'état, ce qu'on suppose atteint.
    pub optimistic: Optimistic,
}

/// Erreur d'un appel de service : le message n'est pas publié.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("paramètre « {0} » manquant ou invalide")]
    Invalid(&'static str),
    #[error("« {field} » doit être entre {min} et {max}")]
    OutOfRange {
        field: &'static str,
        min: String,
        max: String,
    },
    #[error("« {0} » n'est pas un choix proposé par l'appareil")]
    UnknownChoice(String),
    #[error("« {field} » : de {min} à {max} caractères, sans caractère de contrôle")]
    Text {
        field: &'static str,
        min: usize,
        max: usize,
    },
}

/// Ce que sait faire un appareil (vide pour un simple capteur ou interrupteur).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profile {
    /// Valeur reçue → état de l'entité. Vide : marche / arrêt, ou valeur brute.
    pub states: Vec<(String, String)>,
    pub attributes: Vec<AttributeTopic>,
    pub services: Vec<ServiceCommand>,
    /// Attributs fixes (bornes, modes…), annoncés une fois pour toutes.
    pub fixed: Vec<(&'static str, Value)>,
    /// L'état quand il n'est pas dans [`Profile::states`].
    pub free: FreeState,
    /// Les sortes d'événements annoncées (entité `event` : `single`, `double`…).
    pub event_types: Vec<String>,
    /// Une caméra dont l'image arrive en base64 (`image_encoding: b64`).
    pub image_base64: bool,
}

impl Profile {
    /// L'état correspondant à une valeur reçue, s'il est connu.
    #[must_use]
    pub fn state_for(&self, value: &str) -> Option<&str> {
        self.states
            .iter()
            .find(|(received, _)| received == value)
            .map(|(_, state)| state.as_str())
    }

    #[must_use]
    pub fn command(&self, service: &str) -> Option<&ServiceCommand> {
        self.services.iter().find(|c| c.service == service)
    }

    /// Les attributs fixes, à ajouter à chaque état.
    pub fn add_fixed(&self, attributes: &mut Attributes) {
        for (name, value) in &self.fixed {
            attributes.insert((*name).into(), value.clone());
        }
    }
}

impl ServiceCommand {
    /// Le texte à publier, d'après les paramètres du service.
    pub fn render(&self, data: &Attributes) -> Result<String, CommandError> {
        match &self.payload {
            Payload::Fixed(text) => Ok(text.clone()),
            Payload::Number {
                field,
                min,
                max,
                scale,
            } => {
                let value = data
                    .get(*field)
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite())
                    .ok_or(CommandError::Invalid(field))?;
                if value < *min || value > *max {
                    return Err(CommandError::OutOfRange {
                        field,
                        min: format_number(*min),
                        max: format_number(*max),
                    });
                }
                Ok(match scale {
                    Some((low, high)) => {
                        let ratio = if max > min {
                            (value - min) / (max - min)
                        } else {
                            0.0
                        };
                        format_number((low + ratio * (high - low)).round())
                    }
                    None => format_number(value),
                })
            }
            Payload::Flag { field, on, off } => match data.get(*field).and_then(Value::as_bool) {
                Some(true) => Ok(on.clone()),
                Some(false) => Ok(off.clone()),
                None => Err(CommandError::Invalid(field)),
            },
            Payload::Text { field, min, max } => {
                let text = data
                    .get(*field)
                    .and_then(Value::as_str)
                    .ok_or(CommandError::Invalid(field))?;
                let count = text.chars().count();
                if count < *min || count > *max || text.chars().any(char::is_control) {
                    return Err(CommandError::Text {
                        field,
                        min: *min,
                        max: *max,
                    });
                }
                Ok(text.to_owned())
            }
            Payload::Choice { field, options } => {
                let value = data
                    .get(*field)
                    .and_then(Value::as_str)
                    .ok_or(CommandError::Invalid(field))?;
                options
                    .iter()
                    .find(|option| *option == value)
                    .cloned()
                    .ok_or_else(|| CommandError::UnknownChoice(value.chars().take(64).collect()))
            }
        }
    }
}

/// `45` plutôt que `45.0` : beaucoup d'appareils n'acceptent que des entiers.
#[must_use]
pub fn format_number(value: f64) -> String {
    if value == 0.0 {
        // Évite « -0 ».
        "0".to_owned()
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

/// Lit la valeur d'un attribut dans un message, `None` si elle est absente
/// ou n'a pas la forme attendue.
#[must_use]
pub fn attribute_value(topic: &AttributeTopic, payload: &str) -> Option<Value> {
    let text = topic.template.render(payload)?;
    let number = || text.trim().parse::<f64>().ok().filter(|n| n.is_finite());
    match &topic.kind {
        AttrKind::Number => number().map(|n| json!(n)),
        // Même règle que les autres textes : court et sans caractères de contrôle.
        AttrKind::Text => {
            (text.chars().count() <= 64 && !text.chars().any(char::is_control)).then(|| json!(text))
        }
        AttrKind::Flag { on, off } => {
            let text = text.trim();
            if text == on {
                Some(json!(true))
            } else if text == off {
                Some(json!(false))
            } else {
                None
            }
        }
        AttrKind::Percent { min, max } => {
            let value = number()?;
            if max <= min {
                return None;
            }
            let percent = ((value - min) / (max - min) * 100.0)
                .clamp(0.0, 100.0)
                .round();
            Some(json!(percent))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(value: Value) -> Attributes {
        let Value::Object(map) = value else {
            panic!("un objet est attendu")
        };
        map
    }

    #[test]
    fn numbers_are_checked_and_written_simply() {
        let command = ServiceCommand {
            service: "set_humidity",
            topic: "h/set".into(),
            payload: Payload::Number {
                field: "humidity",
                min: 30.0,
                max: 80.0,
                scale: None,
            },
            optimistic: Optimistic::Keep,
        };
        assert_eq!(
            command.render(&data(json!({"humidity": 45}))).unwrap(),
            "45"
        );
        assert_eq!(
            command.render(&data(json!({"humidity": 45.5}))).unwrap(),
            "45.5"
        );
        assert!(matches!(
            command.render(&data(json!({"humidity": 95}))),
            Err(CommandError::OutOfRange { .. })
        ));
        for wrong in [
            json!({}),
            json!({"humidity": "45"}),
            json!({"humidity": null}),
        ] {
            assert_eq!(
                command.render(&data(wrong)),
                Err(CommandError::Invalid("humidity"))
            );
        }
    }

    #[test]
    fn choices_must_be_announced() {
        let command = ServiceCommand {
            service: "set_operation_mode",
            topic: "c/mode".into(),
            payload: Payload::Choice {
                field: "operation_mode",
                options: vec!["eco".into(), "off".into()],
            },
            optimistic: Optimistic::Keep,
        };
        assert_eq!(
            command
                .render(&data(json!({"operation_mode": "eco"})))
                .unwrap(),
            "eco"
        );
        assert!(matches!(
            command.render(&data(json!({"operation_mode": "turbo; rm -rf"}))),
            Err(CommandError::UnknownChoice(_))
        ));
    }

    #[test]
    fn attribute_values() {
        let numeric = AttributeTopic {
            name: "battery_level",
            topic: "v".into(),
            template: Template::parse("{{ value_json.battery_level }}").unwrap(),
            kind: AttrKind::Number,
        };
        assert_eq!(
            attribute_value(&numeric, r#"{"battery_level": 61}"#),
            Some(json!(61.0))
        );
        assert_eq!(
            attribute_value(&numeric, r#"{"battery_level": "beaucoup"}"#),
            None
        );
        let text = AttributeTopic {
            kind: AttrKind::Text,
            ..numeric
        };
        assert_eq!(
            attribute_value(&text, r#"{"battery_level": "ligne\nnouvelle"}"#),
            None
        );
    }

    #[test]
    fn scaled_positions_flags_and_texts() {
        let position = ServiceCommand {
            service: "set_cover_position",
            topic: "v/pos".into(),
            payload: Payload::Number {
                field: "position",
                min: 0.0,
                max: 100.0,
                scale: Some((0.0, 255.0)),
            },
            optimistic: Optimistic::Keep,
        };
        assert_eq!(
            position.render(&data(json!({"position": 50}))).unwrap(),
            "128"
        );
        let read = AttributeTopic {
            name: "current_position",
            topic: "v".into(),
            template: Template::Raw,
            kind: AttrKind::Percent {
                min: 0.0,
                max: 255.0,
            },
        };
        assert_eq!(attribute_value(&read, "255"), Some(json!(100.0)));
        assert_eq!(attribute_value(&read, "300"), Some(json!(100.0)), "borné");

        let oscillate = ServiceCommand {
            service: "oscillate",
            topic: "f/osc".into(),
            payload: Payload::Flag {
                field: "oscillating",
                on: "oscillate_on".into(),
                off: "oscillate_off".into(),
            },
            optimistic: Optimistic::Keep,
        };
        assert_eq!(
            oscillate
                .render(&data(json!({"oscillating": false})))
                .unwrap(),
            "oscillate_off"
        );
        assert!(
            oscillate
                .render(&data(json!({"oscillating": "oui"})))
                .is_err()
        );

        let text = ServiceCommand {
            service: "set_value",
            topic: "t/set".into(),
            payload: Payload::Text {
                field: "value",
                min: 1,
                max: 10,
            },
            optimistic: Optimistic::Sent,
        };
        assert_eq!(
            text.render(&data(json!({"value": "Bonjour"}))).unwrap(),
            "Bonjour"
        );
        for bad in [
            json!({"value": ""}),
            json!({"value": "beaucoup trop long"}),
            json!({"value": "a\u{1b}b"}),
        ] {
            assert!(text.render(&data(bad.clone())).is_err(), "{bad}");
        }
    }
}
