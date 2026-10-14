//! Tâches de fond qu'on sait arrêter proprement.
//!
//! L'enregistreur, MQTT, Hue, les automatisations… tournent tous en arrière-
//! plan et doivent s'arrêter proprement (écrire les derniers événements, se
//! déconnecter). Ils utilisent tous ce même petit outil.

use std::future::Future;

use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};

/// Une tâche de fond, avec de quoi lui demander de s'arrêter.
///
/// Concept : un `CancellationToken` est un « signal d'arrêt » partagé. La
/// tâche attend `token.cancelled()` dans son `select!` ; quand on annule le
/// jeton, cette attente se termine et la tâche sort de sa boucle.
///
/// Concept Rust : on garde un `DropGuard` plutôt que le jeton lui-même. Un
/// `DropGuard` annule le jeton **quand il est détruit**. Donc si la poignée
/// `Background` est abandonnée sans appeler [`Background::stop`] (oubli,
/// erreur qui fait sortir de la fonction…), la tâche s'arrête quand même.
#[derive(Debug)]
pub struct Background {
    name: &'static str,
    guard: DropGuard,
    task: JoinHandle<()>,
}

impl Background {
    /// Lance `run` en arrière-plan. `run` reçoit le jeton à surveiller.
    pub fn spawn<F, Fut>(name: &'static str, run: F) -> Self
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let token = CancellationToken::new();
        let task = tokio::spawn(run(token.clone()));
        Self {
            name,
            guard: token.drop_guard(),
            task,
        }
    }

    /// Demande l'arrêt et attend que la tâche ait fini.
    pub async fn stop(self) {
        // Détruire le garde annule le jeton.
        drop(self.guard);
        if let Err(error) = self.task.await {
            tracing::error!("{} s'est arrêté anormalement : {error}", self.name);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[tokio::test]
    async fn stop_cancels_and_waits() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&finished);
        let task = Background::spawn("test", |token| async move {
            token.cancelled().await;
            flag.store(true, Ordering::SeqCst);
        });
        task.stop().await;
        assert!(
            finished.load(Ordering::SeqCst),
            "stop attend la fin de la tâche"
        );
    }

    #[tokio::test]
    async fn dropping_the_handle_also_stops_the_task() {
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let task = Background::spawn("test", |token| async move {
            token.cancelled().await;
            let _ = done_tx.send(());
        });
        drop(task);
        tokio::time::timeout(std::time::Duration::from_secs(1), done_rx)
            .await
            .expect("la tâche aurait dû s'arrêter")
            .expect("la tâche aurait dû signaler sa fin");
    }
}
