//! Les conversations d'Assist (étape 96), comme dans Home Assistant : la
//! réponse donne un `conversation_id`, que la tablette renvoie avec la
//! phrase suivante (« et la cuisine aussi »).
//!
//! Le serveur garde pour chaque conversation le petit contexte d'Assist
//! ([`ha_assist::suite::Context`] : verbe, valeur, appareils visés), **en
//! mémoire seulement**, une minute après la dernière phrase. Jamais le texte
//! dit, jamais sur le disque. Une conversation appartient au compte qui l'a
//! commencée : un autre compte qui présente son identifiant en commence une
//! nouvelle, sans rien voir de l'autre.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use ha_assist::suite::Context;

/// Une conversation est oubliée une minute après sa dernière phrase.
pub(crate) const KEEP: Duration = Duration::from_secs(60);
/// Conversations gardées à la fois au plus (les plus vieilles partent).
const MAX_CONVERSATIONS: usize = 64;
/// Phrases au plus dans une conversation : ensuite, la réponse dit de ne
/// plus écouter (`continue_conversation: false`).
pub(crate) const MAX_TURNS: u32 = 6;
/// 128 bits aléatoires, en hexadécimal.
const ID_BYTES: usize = 16;

#[derive(Debug)]
struct Kept {
    user_id: i64,
    context: Option<Context>,
    turns: u32,
    until: Instant,
}

/// Les conversations en cours.
#[derive(Debug, Default)]
pub(crate) struct Conversations(Mutex<HashMap<String, Kept>>);

/// La conversation après une phrase, pour la réponse.
pub(crate) struct Next {
    pub(crate) id: Option<String>,
    /// Encore de la place pour une phrase ?
    pub(crate) more: bool,
}

impl Conversations {
    /// Le contexte de cette conversation, si elle existe encore et
    /// appartient à ce compte.
    pub(crate) fn context(&self, id: Option<&str>, user_id: i64) -> Option<Context> {
        let id = id.filter(|id| valid(id))?;
        let map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        map.get(id)
            .filter(|kept| kept.user_id == user_id && kept.until > Instant::now())
            .and_then(|kept| kept.context.clone())
    }

    /// Retient une phrase de plus dans la conversation `id` (ou dans une
    /// nouvelle), avec son nouveau contexte (`None` : garder l'ancien).
    pub(crate) fn keep(&self, id: Option<&str>, user_id: i64, context: Option<Context>) -> Next {
        let now = Instant::now();
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, kept| kept.until > now);
        if let Some(id) = id.filter(|id| valid(id))
            && let Some(kept) = map.get_mut(id)
            && kept.user_id == user_id
        {
            kept.turns += 1;
            kept.until = now + KEEP;
            if context.is_some() {
                kept.context = context;
            }
            return Next {
                id: Some(id.to_owned()),
                more: kept.turns < MAX_TURNS,
            };
        }
        // Une nouvelle conversation.
        let Some(id) = new_id() else {
            return Next {
                id: None,
                more: false,
            };
        };
        if map.len() >= MAX_CONVERSATIONS
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, kept)| kept.until)
                .map(|(id, _)| id.clone())
        {
            map.remove(&oldest);
        }
        map.insert(
            id.clone(),
            Kept {
                user_id,
                context,
                turns: 1,
                until: now + KEEP,
            },
        );
        Next {
            id: Some(id),
            more: true,
        }
    }

    /// Termine la conversation (« merci », « c'est tout ») : son contexte
    /// est oublié tout de suite.
    pub(crate) fn end(&self, id: Option<&str>, user_id: i64) {
        let Some(id) = id.filter(|id| valid(id)) else {
            return;
        };
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if map.get(id).is_some_and(|kept| kept.user_id == user_id) {
            map.remove(id);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// Un identifiant de conversation bien formé : 32 chiffres hexadécimaux.
pub(crate) fn valid(id: &str) -> bool {
    id.len() == 2 * ID_BYTES && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Un nouvel identifiant, tiré du générateur aléatoire du système.
fn new_id() -> Option<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0u8; ID_BYTES];
    if let Err(error) = getrandom::fill(&mut bytes) {
        tracing::warn!("assist : générateur aléatoire indisponible ({error})");
        return None;
    }
    let mut id = String::with_capacity(2 * ID_BYTES);
    for byte in bytes {
        id.push(char::from(HEX[usize::from(byte >> 4)]));
        id.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn identifiers_are_random_and_well_formed() {
        let a = new_id().unwrap();
        let b = new_id().unwrap();
        assert!(valid(&a) && valid(&b));
        assert_ne!(a, b);
        assert!(!valid("../etc"));
        assert!(!valid(&a.to_uppercase()));
        assert!(!valid(&format!("{a}0")));
    }

    #[test]
    fn a_conversation_belongs_to_its_account() {
        let conversations = Conversations::default();
        let first = conversations.keep(None, 1, None).id.unwrap();
        // Le même compte continue la même conversation.
        let again = conversations.keep(Some(&first), 1, None);
        assert_eq!(again.id.as_deref(), Some(first.as_str()));
        // Un autre compte en commence une autre.
        let other = conversations.keep(Some(&first), 2, None).id.unwrap();
        assert_ne!(other, first);
        // Un identifiant inventé aussi.
        let invented = conversations.keep(Some("0123"), 1, None).id.unwrap();
        assert_ne!(invented, first);
    }

    #[test]
    fn the_number_of_sentences_is_limited() {
        let conversations = Conversations::default();
        let id = conversations.keep(None, 1, None).id.unwrap();
        let mut more = true;
        for _ in 1..MAX_TURNS {
            more = conversations.keep(Some(&id), 1, None).more;
        }
        assert!(!more);
    }

    #[test]
    fn old_conversations_make_room() {
        let conversations = Conversations::default();
        for _ in 0..MAX_CONVERSATIONS + 10 {
            conversations.keep(None, 1, None);
        }
        assert_eq!(conversations.len(), MAX_CONVERSATIONS);
    }

    #[test]
    fn ending_forgets_the_conversation() {
        let conversations = Conversations::default();
        let id = conversations.keep(None, 1, None).id.unwrap();
        // Un autre compte ne peut pas la terminer.
        conversations.end(Some(&id), 2);
        assert_eq!(conversations.len(), 1);
        conversations.end(Some(&id), 1);
        assert_eq!(conversations.len(), 0);
    }
}
