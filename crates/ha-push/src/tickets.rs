//! Les boutons des notifications (« Valider », « Refuser », ou ceux d'une
//! automatisation) : un toucher agit sans ouvrir le tableau de bord.
//!
//! Le service worker ne connaît pas le jeton de connexion de la page (il est
//! rangé dans l'onglet). Chaque notification qui a des boutons emporte donc
//! un **ticket** : un secret tiré au hasard, valable pour **ce** message,
//! **cet** appareil et **ce** compte, une seule fois, et peu de temps.
//!
//! - Le ticket voyage dans la notification, chiffrée de bout en bout : le
//!   service de notifications (Google, Apple…) ne le voit pas.
//! - Le serveur n'en garde que l'empreinte (SHA-256), en mémoire : un
//!   redémarrage les oublie tous (les ordres retenus aussi).
//! - Un ticket ne permet que les boutons de son message : valider **cet**
//!   ordre retenu, ou signaler **ce** bouton d'automatisation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};

use crate::crypto;

/// Tickets gardés au plus (les plus proches de leur fin partent d'abord).
const MAX_TICKETS: usize = 500;
/// Octets tirés au hasard pour un ticket (256 bits).
const TICKET_BYTES: usize = 32;

/// À quoi sert un ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    /// Valider ou refuser l'ordre retenu qui porte ce numéro.
    Approval(u64),
    /// Un bouton ajouté par une automatisation : son identifiant est
    /// annoncé sur le bus (événement `notification_action`).
    Automation,
}

/// Ce qu'un ticket permet, et à qui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// L'appareil qui a reçu la notification : le retirer annule le ticket.
    pub subscription: i64,
    pub user_id: i64,
    pub username: String,
    pub purpose: Purpose,
    /// Les identifiants des boutons de la notification.
    pub actions: Vec<String>,
    /// Le titre de la notification, pour la réponse affichée.
    pub title: String,
    expires: Instant,
}

impl Ticket {
    #[must_use]
    pub fn new(
        subscription: i64,
        user_id: i64,
        username: String,
        purpose: Purpose,
        actions: Vec<String>,
        title: String,
        lifetime: Duration,
    ) -> Self {
        Self {
            subscription,
            user_id,
            username,
            purpose,
            actions,
            title,
            expires: Instant::now() + lifetime,
        }
    }
}

/// Pourquoi un ticket est refusé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// Inconnu, déjà servi ou expiré : on ne dit pas lequel.
    #[error("ce bouton n'est plus valable : ouvre le tableau de bord")]
    Unknown,
    /// Ce bouton n'est pas celui de la notification : le ticket reste
    /// valable.
    #[error("bouton inconnu")]
    Action,
}

/// Les tickets en cours, partagés entre l'envoi et l'API.
#[derive(Debug, Clone, Default)]
pub struct Tickets(Arc<Mutex<HashMap<[u8; 32], Ticket>>>);

fn fingerprint(token: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(digest(&SHA256, token.as_bytes()).as_ref());
    out
}

impl Tickets {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<[u8; 32], Ticket>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Range un ticket et rend le secret à mettre dans la notification.
    /// `None` si le hasard du système est indisponible.
    #[must_use]
    pub fn issue(&self, ticket: Ticket) -> Option<String> {
        let mut secret = [0u8; TICKET_BYTES];
        SystemRandom::new().fill(&mut secret).ok()?;
        let token = crypto::encode(&secret);
        let now = Instant::now();
        let mut map = self.map();
        map.retain(|_, kept| kept.expires > now);
        // Plein : les tickets d'automatisation partent d'abord (le plus
        // proche de sa fin) ; ceux d'un ordre à valider sont protégés, sinon
        // un flot de messages chasserait les boutons « Valider ».
        while map.len() >= MAX_TICKETS {
            let victim = map
                .iter()
                .filter(|(_, kept)| kept.purpose == Purpose::Automation)
                .min_by_key(|(_, kept)| kept.expires)
                .or_else(|| map.iter().min_by_key(|(_, kept)| kept.expires))
                .map(|(key, _)| *key)?;
            map.remove(&victim);
        }
        map.insert(fingerprint(&token), ticket);
        Some(token)
    }

    /// Sert un ticket pour le bouton `action` : il est retiré (une seule
    /// fois). Un bouton qui n'est pas celui de la notification ne le
    /// consomme pas.
    pub fn redeem(&self, token: &str, action: &str) -> Result<Ticket, Refused> {
        if token.len() > 100 {
            return Err(Refused::Unknown);
        }
        let key = fingerprint(token);
        let mut map = self.map();
        let ticket = map.get(&key).ok_or(Refused::Unknown)?;
        if ticket.expires <= Instant::now() {
            map.remove(&key);
            return Err(Refused::Unknown);
        }
        if !ticket.actions.iter().any(|known| known == action) {
            return Err(Refused::Action);
        }
        map.remove(&key).ok_or(Refused::Unknown)
    }

    /// Oublie les tickets d'un ordre retenu (il vient d'être traité depuis
    /// un autre appareil, ou depuis la cloche).
    pub fn forget_approval(&self, id: u64) {
        self.map()
            .retain(|_, ticket| ticket.purpose != Purpose::Approval(id));
    }

    /// Combien de tickets sont en cours (pour les essais).
    #[must_use]
    pub fn len(&self) -> usize {
        self.map().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(purpose: Purpose, lifetime: Duration) -> Ticket {
        Ticket::new(
            3,
            1,
            "david".into(),
            purpose,
            vec!["approve".into(), "deny".into()],
            "Ordre à valider".into(),
            lifetime,
        )
    }

    #[test]
    fn a_ticket_is_served_once_and_only_for_its_buttons() {
        let tickets = Tickets::default();
        let token = tickets
            .issue(ticket(Purpose::Approval(7), Duration::from_mins(10)))
            .unwrap();
        assert_eq!(token.len(), 43, "256 bits en base64url");
        assert_eq!(tickets.redeem(&token, "open"), Err(Refused::Action));
        // Le mauvais bouton ne l'a pas consommé.
        let served = tickets.redeem(&token, "approve").unwrap();
        assert_eq!(served.purpose, Purpose::Approval(7));
        assert_eq!(served.username, "david");
        assert_eq!(tickets.redeem(&token, "approve"), Err(Refused::Unknown));
        assert_eq!(tickets.redeem("inventé", "approve"), Err(Refused::Unknown));
        assert_eq!(
            tickets.redeem(&"a".repeat(5000), "approve"),
            Err(Refused::Unknown)
        );
    }

    #[test]
    fn tickets_expire_and_are_bounded() {
        let tickets = Tickets::default();
        let expired = tickets
            .issue(ticket(Purpose::Automation, Duration::ZERO))
            .unwrap();
        assert_eq!(tickets.redeem(&expired, "approve"), Err(Refused::Unknown));
        for _ in 0..MAX_TICKETS + 20 {
            let _ = tickets.issue(ticket(Purpose::Automation, Duration::from_hours(1)));
        }
        assert_eq!(tickets.len(), MAX_TICKETS);
    }

    #[test]
    fn approval_tickets_survive_a_flood_of_automation_tickets() {
        // Revue de la PR #60 : la réserve pleine de tickets d'automatisation
        // (24 h), les tickets d'un ordre à valider (10 min) se chassaient
        // l'un l'autre.
        let tickets = Tickets::default();
        for _ in 0..MAX_TICKETS {
            let _ = tickets.issue(ticket(Purpose::Automation, Duration::from_hours(24)));
        }
        let phones: Vec<String> = (0..4)
            .map(|_| {
                tickets
                    .issue(ticket(Purpose::Approval(9), Duration::from_mins(10)))
                    .unwrap()
            })
            .collect();
        assert_eq!(tickets.len(), MAX_TICKETS);
        for phone in &phones {
            assert!(
                tickets.redeem(phone, "approve").is_ok(),
                "chaque téléphone garde son bouton"
            );
        }
    }

    #[test]
    fn an_answered_approval_cancels_its_other_tickets() {
        let tickets = Tickets::default();
        let phone = tickets
            .issue(ticket(Purpose::Approval(7), Duration::from_mins(10)))
            .unwrap();
        let other = tickets
            .issue(ticket(Purpose::Approval(8), Duration::from_mins(10)))
            .unwrap();
        tickets.forget_approval(7);
        assert_eq!(tickets.redeem(&phone, "approve"), Err(Refused::Unknown));
        assert!(tickets.redeem(&other, "deny").is_ok());
        assert_eq!(tickets.len(), 0);
    }
}
