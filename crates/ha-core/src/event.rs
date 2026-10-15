//! Bus d'événements : tout ce qui se passe dans le système y est publié,
//! et chaque partie intéressée (automatisations, historique, interface…) s'y abonne.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::entity_id::EntityId;
use crate::state::{Attributes, State};

/// Nombre d'événements gardés en mémoire pour un abonné qui prend du retard.
pub const DEFAULT_BUS_CAPACITY: usize = 1024;

/// Un événement.
///
/// Concept Rust : un `enum` Rust est bien plus qu'une liste de constantes :
/// chaque variante peut porter ses propres données. On en ajoutera d'autres
/// (appel de service, démarrage…) dans les prochaines étapes, et le compilateur
/// signalera chaque `match` qui ne les gère pas encore.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// L'état d'une entité a changé.
    /// `old_state` vaut `None` à la création, `new_state` vaut `None` à la suppression.
    StateChanged {
        entity_id: EntityId,
        old_state: Option<State>,
        new_state: Option<State>,
    },

    /// Demande d'action sur une entité, par exemple `switch.turn_on` sur
    /// `switch.prise_cafe`. Le noyau ne sait pas comment allumer une prise :
    /// c'est l'intégration propriétaire de l'entité (MQTT…) qui écoute ces
    /// événements et envoie l'ordre à l'appareil.
    CallService {
        /// Domaine du service : `switch`, `light`…
        domain: String,
        /// Nom du service : `turn_on`, `turn_off`, `toggle`…
        service: String,
        entity_id: EntityId,
        /// Paramètres facultatifs, par exemple `{"brightness": 128}`.
        data: Attributes,
    },

    /// Un événement « libre », nommé : `timer.finished` quand un minuteur
    /// arrive à zéro, par exemple. Les automatisations peuvent s'y abonner
    /// (déclencheur `event`), comme dans Home Assistant.
    Fired {
        /// Le nom : `timer.finished`, `input_button.pressed`…
        event_type: String,
        /// Les détails, par exemple `{"entity_id": "timer.lessive"}`.
        data: Attributes,
    },
}

/// Le bus d'événements.
///
/// Il repose sur un canal « broadcast » de tokio : chaque événement publié est
/// reçu par **tous** les abonnés.
///
/// Concept Rust : `Arc` (« Atomically Reference Counted ») permet à plusieurs
/// abonnés de partager le même événement sans le copier. La mémoire est libérée
/// quand le dernier abonné l'a lâché.
///
/// `#[derive(Clone)]` : cloner le bus ne crée pas un nouveau bus, cela donne une
/// deuxième « poignée » sur le même canal.
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: broadcast::Sender<Arc<Event>>,
}

impl EventBus {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publie un événement et renvoie le nombre d'abonnés qui le recevront.
    #[expect(
        clippy::must_use_candidate,
        reason = "ignorer le nombre d'abonnés est normal : on publie et on continue"
    )]
    pub fn publish(&self, event: Event) -> usize {
        // `send` échoue seulement s'il n'y a aucun abonné : l'événement est alors
        // simplement perdu, ce qui est normal.
        self.sender.send(Arc::new(event)).unwrap_or(0)
    }

    /// S'abonne aux événements publiés **à partir de maintenant**.
    ///
    /// Un abonné trop lent qui laisse passer plus de `capacity` événements reçoit
    /// `RecvError::Lagged` : il a perdu des événements mais peut continuer.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_BUS_CAPACITY)
    }
}
