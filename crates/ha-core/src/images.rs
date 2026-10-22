//! La **dernière image** de chaque caméra (étape 82) : la photo de la
//! sonnette, prise quand on sonne.
//!
//! Gardée en mémoire seulement : jamais écrite dans la base ni dans
//! l'historique. Au redémarrage, la caméra en renvoie une.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use crate::EntityId;

/// Taille maximale d'une image.
pub const MAX_IMAGE: usize = 2 << 20;
/// Caméras gardées au plus.
const MAX_CAMERAS: usize = 32;

/// Une image et ce qu'il faut pour la servir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub bytes: Arc<[u8]>,
    /// `image/jpeg` ou `image/png`.
    pub content_type: &'static str,
    pub at: SystemTime,
}

/// Le type d'une image d'après ses premiers octets, ou `None` si ce n'est
/// ni un JPEG ni un PNG.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else {
        None
    }
}

/// Les images, partagées entre l'intégration qui les reçoit et l'API qui les
/// sert.
#[derive(Debug, Default)]
pub struct Images(Mutex<HashMap<EntityId, Image>>);

impl Images {
    /// Garde l'image (en remplaçant la précédente). Refusée si elle est trop
    /// grosse ou n'est pas une image.
    pub fn set(&self, entity_id: EntityId, bytes: &[u8]) -> Result<&'static str, String> {
        if bytes.len() > MAX_IMAGE {
            return Err(format!("image trop grosse ({} octets)", bytes.len()));
        }
        let content_type = sniff(bytes).ok_or("ni JPEG ni PNG")?;
        let mut images = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !images.contains_key(&entity_id) && images.len() >= MAX_CAMERAS {
            return Err("trop de caméras".into());
        }
        images.insert(
            entity_id,
            Image {
                bytes: bytes.into(),
                content_type,
                at: SystemTime::now(),
            },
        );
        Ok(content_type)
    }

    #[must_use]
    pub fn get(&self, entity_id: &EntityId) -> Option<Image> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(entity_id)
            .cloned()
    }

    /// Oublie l'image (caméra retirée).
    pub fn remove(&self, entity_id: &EntityId) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(entity_id);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn keeps_only_real_images() {
        let images = Images::default();
        let id: EntityId = "camera.porte".parse().unwrap();
        assert!(images.set(id.clone(), b"<html>").is_err());
        assert!(images.set(id.clone(), &vec![0xFF; MAX_IMAGE + 1]).is_err());
        assert_eq!(
            images
                .set(id.clone(), &[0xFF, 0xD8, 0xFF, 0xE0, 1])
                .unwrap(),
            "image/jpeg"
        );
        assert_eq!(images.get(&id).unwrap().bytes.len(), 5);
        images.remove(&id);
        assert!(images.get(&id).is_none());
    }
}
