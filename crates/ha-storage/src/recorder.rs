//! L'enregistreur : une tâche abonnée au bus qui écrit chaque changement d'état
//! dans la base.

use std::sync::Arc;

use ha_core::{Background, CancellationToken, Core, Event};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tracing::{error, warn};

use crate::store::Store;

/// Poignée vers l'enregistreur qui tourne en arrière-plan.
///
/// Si la poignée est abandonnée sans appeler [`Recorder::stop`], l'enregistreur
/// s'arrête aussi (après avoir écrit les événements en attente).
#[derive(Debug)]
pub struct Recorder(Background);

impl Recorder {
    /// Démarre l'enregistreur.
    ///
    /// L'abonnement au bus est fait **ici**, avant de rendre la main : aucun
    /// changement fait juste après cet appel ne peut être manqué.
    pub fn start(store: Store, core: Arc<Core>) -> Self {
        let events = core.bus.subscribe();
        Self(Background::spawn("l'enregistreur", |token| {
            run(store, core, events, token)
        }))
    }

    /// Demande l'arrêt et attend que les derniers événements soient écrits.
    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(
    store: Store,
    core: Arc<Core>,
    mut events: Receiver<Arc<Event>>,
    token: CancellationToken,
) {
    loop {
        // Concept Rust : `select!` attend le premier qui arrive, un événement
        // ou la demande d'arrêt.
        tokio::select! {
            received = events.recv() => match received {
                Ok(event) => record(&store, &event).await,
                Err(RecvError::Lagged(missed)) => resync(&store, &core, missed).await,
                Err(RecvError::Closed) => return,
            },
            () = token.cancelled() => break,
        }
    }

    // Arrêt demandé : on vide ce qui reste dans le canal avant de partir.
    loop {
        match events.try_recv() {
            Ok(event) => record(&store, &event).await,
            Err(TryRecvError::Lagged(missed)) => resync(&store, &core, missed).await,
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
        }
    }
}

async fn record(store: &Store, event: &Event) {
    match event {
        Event::StateChanged {
            entity_id,
            new_state,
            ..
        } => {
            // Une erreur d'écriture (disque plein…) ne doit pas arrêter le
            // système : on la signale et on continue avec l'événement suivant.
            if let Err(error) = store.record_change(entity_id, new_state.as_ref()).await {
                error!("échec d'enregistrement de {entity_id} : {error}");
            }
        }
        // Un appel de service n'est pas un changement d'état : si l'appareil
        // obéit, son nouvel état arrivera par un `StateChanged`.
        Event::CallService { .. } => {}
    }
}

/// L'enregistreur a pris trop de retard et des événements ont été perdus.
/// L'historique de ces événements est perdu, mais on réécrit le dernier état
/// connu de chaque entité pour que la restauration au démarrage reste juste.
async fn resync(store: &Store, core: &Core, missed: u64) {
    warn!("{missed} événements manqués par l'enregistreur : resynchronisation des états");
    if let Err(error) = store.replace_all_states(&core.states.all()).await {
        error!("échec de la resynchronisation : {error}");
    }
}
