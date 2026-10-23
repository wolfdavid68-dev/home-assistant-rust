//! Les **pochettes** de la musique (étape 92) : l'image de l'album qui
//! joue et celles des favoris.
//!
//! Le tableau de bord n'affiche que des images de notre propre serveur :
//! l'intégration va chercher la pochette (chez l'enceinte ou le service de
//! musique), la range ici sous une clé, et l'API la sert à
//! `/api/media_cover/<clé>`.
//!
//! Gardées en mémoire seulement, au plus [`MAX_COVERS`] : la plus ancienne
//! part quand une nouvelle arrive.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use crate::images::{Image, sniff};

/// Taille maximale d'une pochette.
pub const MAX_COVER: usize = 1 << 20;
/// Pochettes gardées au plus.
pub const MAX_COVERS: usize = 128;
/// Longueur d'une clé.
pub const KEY_LENGTH: usize = 32;

/// Une clé valable : [`KEY_LENGTH`] chiffres hexadécimaux en minuscules.
#[must_use]
pub fn is_key(key: &str) -> bool {
    key.len() == KEY_LENGTH
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Les pochettes, partagées entre l'intégration qui les cherche et l'API
/// qui les sert.
#[derive(Debug, Default)]
pub struct Covers(Mutex<VecDeque<(String, Image)>>);

impl Covers {
    /// Garde la pochette. Refusée si la clé est invalide, si elle est trop
    /// grosse ou si ce n'est pas une image.
    pub fn set(&self, key: &str, bytes: &[u8]) -> Result<&'static str, String> {
        if !is_key(key) {
            return Err("clé de pochette invalide".into());
        }
        if bytes.len() > MAX_COVER {
            return Err(format!("pochette trop grosse ({} octets)", bytes.len()));
        }
        let content_type = sniff(bytes).ok_or("ni JPEG ni PNG")?;
        let mut covers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        covers.retain(|(k, _)| k != key);
        while covers.len() >= MAX_COVERS {
            covers.pop_front();
        }
        covers.push_back((
            key.to_owned(),
            Image {
                bytes: bytes.into(),
                content_type,
                at: SystemTime::now(),
            },
        ));
        Ok(content_type)
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<Image> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, image)| image.clone())
    }

    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(k, _)| k == key)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    const JPEG: [u8; 5] = [0xFF, 0xD8, 0xFF, 0xE0, 1];

    fn key(n: usize) -> String {
        format!("{n:032x}")
    }

    #[test]
    fn keeps_only_real_images_under_valid_keys() {
        let covers = Covers::default();
        assert!(covers.set("../etc", &JPEG).is_err());
        assert!(covers.set(&"AB".repeat(KEY_LENGTH / 2), &JPEG).is_err());
        assert!(covers.set(&key(1)[1..], &JPEG).is_err());
        assert!(covers.set(&key(2), b"<html>").is_err());
        assert!(covers.set(&key(2), &vec![0xFF; MAX_COVER + 1]).is_err());
        assert_eq!(covers.set(&key(2), &JPEG).unwrap(), "image/jpeg");
        assert!(covers.contains(&key(2)));
        assert_eq!(covers.get(&key(2)).unwrap().bytes.len(), 5);
        assert!(covers.get(&key(3)).is_none());
    }

    #[test]
    fn the_oldest_leaves_first() {
        let covers = Covers::default();
        for n in 0..=MAX_COVERS {
            covers.set(&key(n), &JPEG).unwrap();
        }
        assert!(!covers.contains(&key(0)));
        assert!(covers.contains(&key(1)));
        assert!(covers.contains(&key(MAX_COVERS)));
        // La remettre la rend la plus récente.
        covers.set(&key(1), &JPEG).unwrap();
        covers.set(&key(MAX_COVERS + 1), &JPEG).unwrap();
        assert!(covers.contains(&key(1)));
        assert!(!covers.contains(&key(2)));
    }
}
