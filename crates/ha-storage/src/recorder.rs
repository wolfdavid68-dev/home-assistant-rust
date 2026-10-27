//! L'enregistreur : une tâche abonnée au bus qui écrit chaque changement d'état
//! dans la base.

use std::sync::Arc;

use ha_core::{Background, CancellationToken, Core, Event};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tracing::{error, warn};

use crate::store::Store;

/// Nombre maximal d'événements écrits dans une même transaction.
///
/// Assez pour absorber une rafale (des dizaines de capteurs qui changent
/// ensemble), assez peu pour ne pas bloquer la base longtemps.
const MAX_BATCH: usize = 256;

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
    let mut batch = Vec::with_capacity(MAX_BATCH);
    loop {
        // Concept Rust : `select!` attend le premier qui arrive, un événement
        // ou la demande d'arrêt.
        tokio::select! {
            received = events.recv() => match received {
                Ok(event) => {
                    // Ceux qui attendent déjà derrière partent avec lui, dans
                    // la même transaction.
                    batch.push(event);
                    let missed = take_pending(&mut events, &mut batch);
                    record_batch(&store, &mut batch).await;
                    if let Some(missed) = missed {
                        resync(&store, &core, missed).await;
                    }
                }
                Err(RecvError::Lagged(missed)) => resync(&store, &core, missed).await,
                Err(RecvError::Closed) => return,
            },
            () = token.cancelled() => break,
        }
    }

    // Arrêt demandé : on vide ce qui reste dans le canal avant de partir.
    loop {
        let missed = take_pending(&mut events, &mut batch);
        if batch.is_empty() && missed.is_none() {
            break;
        }
        record_batch(&store, &mut batch).await;
        if let Some(missed) = missed {
            resync(&store, &core, missed).await;
        }
    }
}

/// Ajoute au lot les événements déjà arrivés, sans attendre, jusqu'à
/// [`MAX_BATCH`]. Renvoie le nombre d'événements perdus si l'enregistreur a
/// pris trop de retard : le lot est alors à écrire **avant** de resynchroniser.
fn take_pending(events: &mut Receiver<Arc<Event>>, batch: &mut Vec<Arc<Event>>) -> Option<u64> {
    while batch.len() < MAX_BATCH {
        match events.try_recv() {
            Ok(event) => batch.push(event),
            Err(TryRecvError::Lagged(missed)) => return Some(missed),
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
        }
    }
    None
}

/// Écrit les changements d'état du lot, dans l'ordre, puis vide le lot.
async fn record_batch(store: &Store, batch: &mut Vec<Arc<Event>>) {
    let changes: Vec<_> = batch
        .iter()
        .filter_map(|event| match event.as_ref() {
            Event::StateChanged {
                entity_id,
                new_state,
                ..
            } => Some((entity_id, new_state.as_deref())),
            // Un appel de service n'est pas un changement d'état : si
            // l'appareil obéit, son nouvel état arrivera par un `StateChanged`.
            // Pareil pour un événement nommé (`timer.finished`…).
            Event::CallService { .. } | Event::Fired { .. } => None,
        })
        .collect();
    if let Err(error) = store.record_changes(&changes).await {
        if let [(entity_id, _)] = changes.as_slice() {
            error!("échec d'enregistrement de {entity_id} : {error}");
        } else {
            // La transaction groupée a été annulée en entier : on réessaie
            // un par un, pour ne perdre que le changement fautif.
            warn!(
                "échec d'enregistrement groupé ({} changements) : {error} ; nouvel essai un par un",
                changes.len()
            );
            for (entity_id, new_state) in changes {
                // Une erreur d'écriture (disque plein…) ne doit pas arrêter le
                // système : on la signale et on continue avec la suivante.
                if let Err(error) = store.record_change(entity_id, new_state).await {
                    error!("échec d'enregistrement de {entity_id} : {error}");
                }
            }
        }
    }
    batch.clear();
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
