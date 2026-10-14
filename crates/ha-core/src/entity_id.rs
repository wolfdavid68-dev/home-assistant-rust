//! Identifiant d'entité, par exemple `light.salon` ou `sensor.temperature_cuisine`.

use std::fmt;
use std::str::FromStr;

use crate::error::CoreError;

/// Longueur maximale d'un identifiant, en octets.
pub const MAX_ENTITY_ID_LEN: usize = 255;

/// Identifiant d'entité au format `domaine.nom`.
///
/// Concept Rust : c'est un « newtype », une structure qui enveloppe une `String`.
/// La seule façon d'en créer un passe par [`EntityId::new`], qui valide le texte.
/// Donc partout ailleurs dans le code, un `EntityId` est forcément valide : le
/// compilateur garantit qu'on n'a jamais oublié la vérification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(String);

impl EntityId {
    /// Valide et crée un identifiant.
    ///
    /// Règles (les mêmes que Home Assistant) : un seul point, de chaque côté
    /// uniquement `a-z`, `0-9` et `_`, pas de `_` au début ni à la fin, pas de
    /// `__` dans le domaine.
    pub fn new(value: &str) -> Result<Self, CoreError> {
        let invalid = || CoreError::InvalidEntityId(value.to_owned());

        if value.len() > MAX_ENTITY_ID_LEN {
            return Err(invalid());
        }
        let (domain, object_id) = value.split_once('.').ok_or_else(invalid)?;
        if !is_valid_part(domain) || !is_valid_part(object_id) || domain.contains("__") {
            return Err(invalid());
        }
        Ok(Self(value.to_owned()))
    }

    /// Le domaine : `light` dans `light.salon`.
    pub fn domain(&self) -> &str {
        // L'identifiant a été validé dans `new`, il contient donc forcément un point.
        self.0.split_once('.').map_or("", |(domain, _)| domain)
    }

    /// Le nom de l'objet : `salon` dans `light.salon`.
    pub fn object_id(&self) -> &str {
        self.0
            .split_once('.')
            .map_or("", |(_, object_id)| object_id)
    }

    /// L'identifiant complet sous forme de texte.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_part(part: &str) -> bool {
    !part.is_empty()
        && !part.starts_with('_')
        && !part.ends_with('_')
        && part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Concept Rust : implémenter le « trait » `Display` permet d'afficher un
/// `EntityId` avec `println!("{id}")`. Un trait est un comportement partagé,
/// proche d'une interface dans d'autres langages.
impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Permet d'écrire `"light.salon".parse::<EntityId>()`.
impl FromStr for EntityId {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Transforme un nom en morceau d'identifiant : `Température Salon` → `temperature_salon`.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(fold_accent) {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('_') && !slug.is_empty() {
            slug.push('_');
        }
    }
    let slug = slug.trim_end_matches('_');
    slug.chars()
        .take(100)
        .collect::<String>()
        .trim_end_matches('_')
        .to_owned()
}

/// Retire les accents les plus courants (é → e, ç → c…).
fn fold_accent(c: char) -> Option<char> {
    Some(match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'À' | 'Â' | 'Ä' | 'Á' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'î' | 'ï' | 'í' | 'Î' | 'Ï' => 'i',
        'ô' | 'ö' | 'ó' | 'õ' | 'Ô' | 'Ö' => 'o',
        'ù' | 'û' | 'ü' | 'ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        'ÿ' => 'y',
        'œ' | 'Œ' => 'o',
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_ids() {
        for id in [
            "light.salon",
            "sensor.temperature_2",
            "switch.prise_cafe",
            "a.b",
        ] {
            assert!(EntityId::new(id).is_ok(), "{id} devrait être valide");
        }
    }

    #[test]
    fn rejects_invalid_ids() {
        for id in [
            "",
            "salon",
            ".salon",
            "light.",
            "light.salon.bis",
            "Light.salon",
            "light.Salon",
            "light.salon-1",
            "light.salon ",
            "_light.salon",
            "light._salon",
            "light.salon_",
            "my__light.salon",
            "light.salon\n",
            "lumière.salon",
        ] {
            assert!(EntityId::new(id).is_err(), "{id:?} devrait être refusé");
        }
    }

    #[test]
    fn rejects_too_long_ids() {
        let id = format!("sensor.{}", "a".repeat(MAX_ENTITY_ID_LEN));
        assert!(EntityId::new(&id).is_err());
    }

    #[test]
    fn splits_domain_and_object_id() {
        let id: EntityId = "light.salon".parse().unwrap();
        assert_eq!(id.domain(), "light");
        assert_eq!(id.object_id(), "salon");
        assert_eq!(id.to_string(), "light.salon");
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Température Salon"), "temperature_salon");
        assert_eq!(slugify("  Prise -- café!! "), "prise_cafe");
        assert_eq!(slugify("0x00158d0001"), "0x00158d0001");
        assert_eq!(slugify("___"), "");
        assert_eq!(slugify("日本"), "");
    }
}
