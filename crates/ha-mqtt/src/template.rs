//! Modèles de valeur (`value_template`) : comment extraire la valeur utile
//! d'un message MQTT.
//!
//! Home Assistant accepte n'importe quel modèle Jinja2, un vrai petit langage
//! de programmation. On n'en accepte volontairement qu'un sous-ensemble très
//! simple, qui couvre Zigbee2MQTT, Tasmota, Shelly et ESPHome :
//!
//! - `{{ value }}` : le message tel quel ;
//! - `{{ value_json.temperature }}` : un champ d'un message JSON ;
//! - `{{ value_json['DS18B20'].Temperature }}`, `{{ value_json.sensors[0] }}` :
//!   des champs imbriqués.
//!
//! Sécurité : un message venu du réseau ne peut ainsi jamais faire exécuter
//! de code, seulement lire un champ.

use serde_json::Value;

/// Un modèle analysé, prêt à être appliqué à chaque message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Template {
    /// `{{ value }}` (ou pas de modèle du tout).
    Raw,
    /// `{{ value_json…}}` suivi d'un chemin.
    Json(Vec<Segment>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

impl Template {
    /// Analyse un modèle. `None` si on ne le comprend pas.
    pub fn parse(template: &str) -> Option<Self> {
        let inner = template
            .trim()
            .strip_prefix("{{")?
            .strip_suffix("}}")?
            .trim();
        if inner == "value" {
            return Some(Self::Raw);
        }
        let mut rest = inner.strip_prefix("value_json")?;
        let mut path = Vec::new();
        while !rest.is_empty() {
            if let Some(after_dot) = rest.strip_prefix('.') {
                // `.nom` : lettres, chiffres et `_`.
                let end = after_dot
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after_dot.len());
                if end == 0 {
                    return None;
                }
                path.push(Segment::Key(after_dot[..end].to_owned()));
                rest = &after_dot[end..];
            } else {
                // `[…]`, sinon c'est un filtre (`| float`…), une opération ou
                // autre chose qu'on ne prend pas en charge.
                let after_bracket = rest.strip_prefix('[')?;
                let end = after_bracket.find(']')?;
                let content = after_bracket[..end].trim();
                let segment = match content.as_bytes() {
                    [b'\'', .., b'\''] | [b'"', .., b'"'] if content.len() >= 2 => {
                        Segment::Key(content[1..content.len() - 1].to_owned())
                    }
                    _ => Segment::Index(content.parse().ok()?),
                };
                path.push(segment);
                rest = &after_bracket[end + 1..];
            }
        }
        Some(Self::Json(path))
    }

    /// Applique le modèle à un message. `None` si la valeur est absente
    /// (champ manquant, JSON invalide, valeur `null`).
    pub fn render(&self, payload: &str) -> Option<String> {
        match self {
            Self::Raw => Some(payload.trim().to_owned()),
            Self::Json(path) => {
                let root: Value = serde_json::from_str(payload).ok()?;
                let mut current = &root;
                for segment in path {
                    current = match segment {
                        Segment::Key(key) => current.get(key)?,
                        Segment::Index(index) => current.get(index)?,
                    };
                }
                match current {
                    Value::Null => None,
                    Value::String(text) => Some(text.clone()),
                    other => Some(other.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    fn render(template: &str, payload: &str) -> Option<String> {
        Template::parse(template)?.render(payload)
    }

    #[test]
    fn zigbee2mqtt_style() {
        let payload = r#"{"temperature": 21.5, "humidity": 48, "battery": null, "state": "ON"}"#;
        assert_eq!(
            render("{{ value_json.temperature }}", payload).as_deref(),
            Some("21.5")
        );
        assert_eq!(
            render("{{value_json.state}}", payload).as_deref(),
            Some("ON")
        );
        assert_eq!(render("{{ value_json.battery }}", payload), None);
        assert_eq!(render("{{ value_json.absent }}", payload), None);
    }

    #[test]
    fn tasmota_style_nested() {
        let payload = r#"{"DS18B20": {"Temperature": 19.2}, "list": [5, 6]}"#;
        assert_eq!(
            render("{{ value_json['DS18B20'].Temperature }}", payload).as_deref(),
            Some("19.2")
        );
        assert_eq!(
            render(r#"{{ value_json["list"][1] }}"#, payload).as_deref(),
            Some("6")
        );
    }

    #[test]
    fn raw_value() {
        assert_eq!(render("{{ value }}", " 42 \n").as_deref(), Some("42"));
    }

    #[test]
    fn unsupported_templates_are_refused() {
        for template in [
            "",
            "value_json.temperature",
            "{{ value_json.temperature | float }}",
            "{{ value_json.a + 1 }}",
            "{% if value == 'ON' %}on{% endif %}",
            "{{ value_json. }}",
            "{{ value_json[abc] }}",
            "{{ states('sensor.x') }}",
        ] {
            assert_eq!(Template::parse(template), None, "{template:?}");
        }
    }

    #[test]
    fn invalid_json_gives_nothing() {
        assert_eq!(render("{{ value_json.a }}", "pas du json"), None);
    }
}
