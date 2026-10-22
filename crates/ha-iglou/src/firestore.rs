//! Les valeurs de Firestore, telles que son API REST les écrit :
//! `{"stringValue": "Lait"}`, `{"integerValue": "2"}` (un texte !),
//! `{"doubleValue": 1.5}`, `{"booleanValue": true}`, `{"nullValue": null}`.

use serde_json::{Map, Value, json};

/// Un document lu : son identifiant (la fin de son chemin) et ses champs.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: String,
    pub fields: Map<String, Value>,
}

impl Document {
    /// Lit un document de l'API (`{"name": "projects/…/aliments/abc", "fields": {…}}`).
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let name = value.get("name")?.as_str()?;
        let id = name.rsplit('/').next()?.to_owned();
        if !is_id(&id) {
            return None;
        }
        let fields = value
            .get("fields")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        Some(Self { id, fields })
    }

    /// Un texte (`None` s'il manque ou n'est pas un texte).
    #[must_use]
    pub fn text(&self, field: &str) -> Option<&str> {
        self.fields.get(field)?.get("stringValue")?.as_str()
    }

    /// Un nombre, entier ou décimal.
    #[must_use]
    pub fn number(&self, field: &str) -> Option<f64> {
        let value = self.fields.get(field)?;
        // Un entier arrive en texte (« "2" ») : lu directement en nombre.
        if let Some(text) = value.get("integerValue").and_then(Value::as_str) {
            return text
                .parse::<i64>()
                .ok()
                .and_then(|_| text.parse::<f64>().ok());
        }
        if let Some(n) = value.get("integerValue").and_then(Value::as_f64) {
            return Some(n);
        }
        value
            .get("doubleValue")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
    }

    /// Un booléen.
    #[must_use]
    pub fn boolean(&self, field: &str) -> Option<bool> {
        self.fields.get(field)?.get("booleanValue")?.as_bool()
    }
}

/// Un identifiant de document sûr à remettre dans une adresse : lettres,
/// chiffres, `-` et `_` (ceux de Firestore en ont 20).
#[must_use]
pub fn is_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[must_use]
pub fn string(text: &str) -> Value {
    json!({ "stringValue": text })
}

#[must_use]
pub fn boolean(value: bool) -> Value {
    json!({ "booleanValue": value })
}

/// Un nombre : entier s'il l'est (comme l'appli), sinon décimal.
#[must_use]
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "entier vérifié juste avant"
        )]
        let whole = value as i64;
        json!({ "integerValue": whole.to_string() })
    } else {
        json!({ "doubleValue": value })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn reads_the_values_of_a_document() {
        let doc = Document::from_json(&json!({
            "name": "projects/p/databases/(default)/documents/maisons/m1/aliments/abc_DEF-1",
            "fields": {
                "nom": {"stringValue": "Lait"},
                "quantite": {"integerValue": "2"},
                "quantiteMax": {"doubleValue": 2.5},
                "dansPanier": {"booleanValue": true},
                "achatQuantite": {"nullValue": null}
            }
        }))
        .unwrap();
        assert_eq!(doc.id, "abc_DEF-1");
        assert_eq!(doc.text("nom"), Some("Lait"));
        assert_eq!(doc.number("quantite"), Some(2.0));
        assert_eq!(doc.number("quantiteMax"), Some(2.5));
        assert_eq!(doc.number("achatQuantite"), None);
        assert_eq!(doc.boolean("dansPanier"), Some(true));
        assert_eq!(doc.text("absent"), None);
    }

    #[test]
    fn refuses_a_strange_identifier() {
        assert!(Document::from_json(&json!({"name": "a/b/../x?y"})).is_none());
        assert!(!is_id(""));
        assert!(!is_id("a/b"));
        assert!(is_id("Ab0_-"));
    }

    #[test]
    fn writes_numbers_like_the_app() {
        assert_eq!(number(2.0), json!({"integerValue": "2"}));
        assert_eq!(number(1.5), json!({"doubleValue": 1.5}));
    }
}
