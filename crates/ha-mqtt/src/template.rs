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
//!   des champs imbriqués ;
//! - `{{ value_json["batteryLevel"] | default("") }}` (ring-mqtt) : la valeur
//!   à prendre si le champ manque, un texte entre guillemets ou un nombre.
//!   `default("")` veut dire « pas de valeur », comme un champ absent.
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
    /// `{{ value_json… | default(…) }}` : le chemin, et la valeur à prendre
    /// si le champ manque (ou si le message n'est pas du JSON).
    JsonOr(Vec<Segment>, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

impl Template {
    /// Analyse un modèle. `None` si on ne le comprend pas.
    #[must_use]
    pub fn parse(template: &str) -> Option<Self> {
        let inner = template
            .trim()
            .strip_prefix("{{")?
            .strip_suffix("}}")?
            .trim();
        let (inner, default) = split_default(inner)?;
        if inner == "value" {
            // `{{ value | default(…) }}` n'a pas de sens : le message est là.
            return default.is_none().then_some(Self::Raw);
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
        Some(match default {
            None => Self::Json(path),
            Some(default) => Self::JsonOr(path, default),
        })
    }

    /// Applique le modèle à un message. `None` si la valeur est absente
    /// (champ manquant, JSON invalide, valeur `null`).
    #[must_use]
    pub fn render(&self, payload: &str) -> Option<String> {
        match self {
            Self::Raw => Some(payload.trim().to_owned()),
            Self::Json(path) => as_text(lookup(path, payload)?),
            Self::JsonOr(path, default) => match lookup(path, payload) {
                Some(value) => as_text(value),
                None => (!default.is_empty()).then(|| default.clone()),
            },
        }
    }
}

/// Sépare un filtre final `| default(…)` du reste du modèle. `None` pour tout
/// autre filtre (`| float`…), refusé comme avant. L'argument est un texte
/// entre guillemets (sans guillemet ni `\` dedans) ou un nombre.
fn split_default(inner: &str) -> Option<(&str, Option<String>)> {
    let Some((expression, filter)) = inner.rsplit_once('|') else {
        return Some((inner, None));
    };
    let argument = filter
        .trim()
        .strip_prefix("default")?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')?
        .trim();
    let default = match argument.as_bytes() {
        [b'\'', .., b'\''] | [b'"', .., b'"'] if argument.len() >= 2 => {
            let text = &argument[1..argument.len() - 1];
            if text.contains(['\'', '"', '\\']) {
                return None;
            }
            text.to_owned()
        }
        _ => {
            argument
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())?;
            argument.to_owned()
        }
    };
    Some((expression.trim_end(), Some(default)))
}

/// Suit le chemin dans le message JSON. `None` si le message n'est pas du
/// JSON ou si un champ du chemin manque.
fn lookup(path: &[Segment], payload: &str) -> Option<Value> {
    let mut current: Value = serde_json::from_str(payload).ok()?;
    for segment in path {
        current = match segment {
            Segment::Key(key) => current.get_mut(key)?.take(),
            Segment::Index(index) => current.get_mut(*index)?.take(),
        };
    }
    Some(current)
}

/// La valeur en texte ; `null` n'en est pas une.
fn as_text(value: Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
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
            "{{ value | default('x') }}",
            "{{ value_json.a | default }}",
            "{{ value_json.a | default(x) }}",
            "{{ value_json.a | default('a', true) }}",
            "{{ value_json.a | default('a\'b') }}",
            "{{ value_json.a | default(\"a\" ~ \"b\") }}",
            "{{ value_json.a | default('') | float }}",
            "{{ value_json.a | float | default(0) }}",
            "{{ value_json.a | default(nan) }}",
        ] {
            assert_eq!(Template::parse(template), None, "{template:?}");
        }
    }

    #[test]
    fn default_filter_like_ring_mqtt() {
        let battery = r#"{{ value_json["batteryLevel"] | default("") }}"#;
        assert_eq!(
            render(
                battery,
                r#"{"batteryLevel": 28, "firmwareStatus": "Up to Date"}"#
            )
            .as_deref(),
            Some("28")
        );
        // Champ absent ou message illisible : « "" », c'est-à-dire rien.
        assert_eq!(render(battery, r#"{"firmwareStatus": "Up to Date"}"#), None);
        assert_eq!(render(battery, "pas du json"), None);
        assert_eq!(render(battery, r#"{"batteryLevel": null}"#), None);
    }

    #[test]
    fn default_filter_values() {
        let payload = r#"{"a": {"b": "ok"}}"#;
        assert_eq!(
            render("{{value_json.a.b|default('x')}}", payload).as_deref(),
            Some("ok")
        );
        assert_eq!(
            render("{{ value_json.a.c | default('inconnu') }}", payload).as_deref(),
            Some("inconnu")
        );
        assert_eq!(
            render("{{ value_json.z.c | default(0) }}", payload).as_deref(),
            Some("0")
        );
        assert_eq!(
            render("{{ value_json.z | default( -1.5 ) }}", payload).as_deref(),
            Some("-1.5")
        );
    }

    #[test]
    fn invalid_json_gives_nothing() {
        assert_eq!(render("{{ value_json.a }}", "pas du json"), None);
    }
}
