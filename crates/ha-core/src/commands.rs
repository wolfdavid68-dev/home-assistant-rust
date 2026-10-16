//! Les ordres avec réponse.
//!
//! Un ordre (`light.turn_on` sur la lampe du salon) part sur le bus. Jusqu'ici,
//! personne ne savait s'il avait abouti : la tuile « clignotait », puis rien.
//! Maintenant :
//!
//! - chaque ordre porte un **numéro** et sa **date d'émission** ;
//! - une intégration qui sait répondre (Hue, Somfy) dit si l'appareil a
//!   accepté ou refusé ([`Core::command_done`]) ; celui qui a donné l'ordre
//!   attend cette réponse quelques secondes ;
//! - un ordre **périmé** (plus de [`COMMAND_TTL`]) n'est jamais exécuté : une
//!   intégration qui se reconnecte après une coupure ne doit pas ouvrir la
//!   porte du garage qu'on lui a demandé d'ouvrir il y a deux minutes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::Core;

/// Au-delà, un ordre est périmé : il n'est plus exécuté.
pub const COMMAND_TTL: Duration = Duration::from_secs(5);
/// Ordres en attente de réponse au plus (une page qui insiste ne doit pas
/// remplir la mémoire) : au-delà, le plus ancien est oublié.
const MAX_PENDING: usize = 256;

/// La réponse d'une intégration : `Ok`, ou le refus de l'appareil.
pub type CommandResult = Result<(), String>;

/// Les ordres en attente de réponse.
#[derive(Debug, Default)]
pub struct Commands {
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<CommandResult>>>,
}

impl Commands {
    /// Un nouveau numéro d'ordre, et de quoi attendre sa réponse.
    pub(crate) fn open(&self) -> (u64, oneshot::Receiver<CommandResult>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = oneshot::channel();
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.len() >= MAX_PENDING
            && let Some(oldest) = pending.keys().min().copied()
        {
            pending.remove(&oldest);
        }
        pending.insert(id, sender);
        (id, receiver)
    }

    fn close(&self, id: u64) -> Option<oneshot::Sender<CommandResult>> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id)
    }
}

impl Core {
    /// Une intégration dit ce qu'est devenu l'ordre `command` (le numéro
    /// porté par [`crate::Event::CallService`]). Sans numéro, ou si plus
    /// personne n'attend, il ne se passe rien.
    pub fn command_done(&self, command: Option<u64>, result: CommandResult) {
        if let Some(sender) = command.and_then(|id| self.commands.close(id)) {
            let _ = sender.send(result);
        }
    }

    /// Celui qui attendait la réponse renonce (délai dépassé).
    pub fn forget_command(&self, id: u64) {
        self.commands.close(id);
    }
}

/// L'ordre a-t-il dépassé [`COMMAND_TTL`] ?
#[must_use]
pub fn is_stale(issued_at: Instant) -> bool {
    issued_at.elapsed() > COMMAND_TTL
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;
    use crate::{Attributes, EntityId, Event, Origin};

    fn lamp() -> EntityId {
        "light.salon".parse().unwrap()
    }

    #[tokio::test]
    async fn an_answer_reaches_whoever_gave_the_order() {
        let core = Core::new();
        let mut events = core.bus.subscribe();
        let (id, reply) = core
            .call_service_tracked(
                Origin::System,
                "light",
                "turn_on",
                lamp(),
                Attributes::new(),
            )
            .unwrap();
        let event = events.recv().await.unwrap();
        let Event::CallService {
            command, issued_at, ..
        } = event.as_ref()
        else {
            panic!("un ordre était attendu");
        };
        assert_eq!(*command, Some(id));
        assert!(!is_stale(*issued_at));
        core.command_done(*command, Err("pont injoignable".into()));
        assert_eq!(reply.await.unwrap(), Err("pont injoignable".to_owned()));
    }

    #[tokio::test]
    async fn an_order_nobody_waits_for_anymore_is_forgotten() {
        let core = Core::new();
        let (id, reply) = core
            .call_service_tracked(
                Origin::System,
                "light",
                "turn_on",
                lamp(),
                Attributes::new(),
            )
            .unwrap();
        core.forget_command(id);
        // La réponse arrive trop tard : elle ne va nulle part.
        core.command_done(Some(id), Ok(()));
        assert!(reply.await.is_err());
        // Sans numéro (ordre d'une automatisation), rien à faire.
        core.command_done(None, Ok(()));
    }

    #[test]
    fn old_orders_are_stale() {
        assert!(!is_stale(Instant::now()));
        assert!(is_stale(
            Instant::now()
                .checked_sub(COMMAND_TTL + Duration::from_millis(1))
                .unwrap()
        ));
    }
}
