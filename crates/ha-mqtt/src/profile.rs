//! Ce qu'un type d'appareil sait faire au-delà de « marche / arrêt » :
//! verrou, vanne, aspirateur, humidificateur, chauffe-eau.
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

/// Un sujet qui donne un attribut de l'entité (`humidity`, `battery_level`…).
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeTopic {
    pub name: &'static str,
    pub topic: String,
    pub template: Template,
    /// Un nombre est attendu : autre chose est ignoré.
    pub numeric: bool,
}

/// Ce qu'on publie pour un service.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// Toujours le même texte (`LOCK`, `start`…).
    Fixed(String),
    /// Un nombre lu dans les paramètres du service, entre deux bornes.
    Number {
        field: &'static str,
        min: f64,
        max: f64,
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
    /// Sans sujet d'état, l'état qu'on suppose atteint (mode « optimiste »).
    pub optimistic: Option<&'static str>,
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
            Payload::Number { field, min, max } => {
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
                Ok(format_number(value))
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
    if topic.numeric {
        let number = text.trim().parse::<f64>().ok().filter(|n| n.is_finite())?;
        Some(json!(number))
    } else {
        // Même règle que les autres textes : court et sans caractères de contrôle.
        (text.chars().count() <= 64 && !text.chars().any(char::is_control)).then(|| json!(text))
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
            },
            optimistic: None,
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
            optimistic: None,
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
            numeric: true,
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
            numeric: false,
            ..numeric
        };
        assert_eq!(
            attribute_value(&text, r#"{"battery_level": "ligne\nnouvelle"}"#),
            None
        );
    }
}
