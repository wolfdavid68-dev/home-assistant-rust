//! Tests du stockage, sur une base SQLite en mémoire (ou un fichier temporaire).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ha_core::{Attributes, Core, EntityId};
use ha_storage::{Recorder, Store};
use serde_json::json;

fn id(value: &str) -> EntityId {
    value.parse().unwrap()
}

fn attrs(value: serde_json::Value) -> Attributes {
    match value {
        serde_json::Value::Object(map) => map,
        other => panic!("objet JSON attendu, reçu {other}"),
    }
}

/// Démarre un noyau et un enregistreur sur une base en mémoire.
async fn setup() -> (Arc<Core>, Store, Recorder) {
    let store = Store::open_in_memory().await.unwrap();
    let core = Arc::new(Core::new());
    let recorder = Recorder::start(store.clone(), Arc::clone(&core));
    (core, store, recorder)
}

/// Fichier temporaire unique, supprimé (avec `-wal` et `-shm`) à la fin du test.
struct TempDb(PathBuf);

impl TempDb {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("ha-test-{}-{name}.db", std::process::id()));
        let db = Self(path);
        db.cleanup();
        db
    }

    fn cleanup(&self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.0.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[tokio::test]
async fn recorder_saves_states_and_history() {
    let (core, store, recorder) = setup().await;
    let light = id("light.salon");

    core.states
        .set(light.clone(), "on", attrs(json!({"brightness": 100})))
        .unwrap();
    core.states
        .set(light.clone(), "on", attrs(json!({"brightness": 200})))
        .unwrap();
    core.states
        .set(light.clone(), "off", Attributes::new())
        .unwrap();
    recorder.stop().await;

    let states = store.load_states().await.unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].entity_id, light);
    assert_eq!(states[0].state, "off");

    // Du plus récent au plus ancien.
    let history = store.history(&light, 10).await.unwrap();
    let values: Vec<(&str, Option<&serde_json::Value>)> = history
        .iter()
        .map(|entry| {
            let state = entry.state.as_ref().unwrap();
            (state.state.as_str(), state.attributes.get("brightness"))
        })
        .collect();
    assert_eq!(
        values,
        [
            ("off", None),
            ("on", Some(&json!(200))),
            ("on", Some(&json!(100))),
        ]
    );
}

#[tokio::test]
async fn removed_entity_leaves_a_trace_in_history() {
    let (core, store, recorder) = setup().await;
    let plug = id("switch.prise_cafe");

    core.states
        .set(plug.clone(), "on", Attributes::new())
        .unwrap();
    core.states.remove(&plug);
    recorder.stop().await;

    assert!(store.load_states().await.unwrap().is_empty());
    let history = store.history(&plug, 10).await.unwrap();
    assert_eq!(history.len(), 2);
    assert!(
        history[0].state.is_none(),
        "la suppression est la plus récente"
    );
    assert_eq!(history[1].state.as_ref().unwrap().state, "on");
}

#[tokio::test]
async fn states_are_restored_with_their_dates_and_without_events() {
    let (core, store, recorder) = setup().await;
    let sensor = id("sensor.temperature_salon");
    core.states
        .set(
            sensor.clone(),
            "21.5",
            attrs(json!({"unit_of_measurement": "°C"})),
        )
        .unwrap();
    let original = core.states.get(&sensor).unwrap();
    recorder.stop().await;

    // « Redémarrage » : un noyau neuf, rechargé depuis la base.
    let restarted = Core::new();
    let mut events = restarted.bus.subscribe();
    let restored = store.restore_into(&restarted.states).await.unwrap();

    assert_eq!(restored, 1);
    let state = restarted.states.get(&sensor).unwrap();
    assert_eq!(state.state(), "21.5");
    assert_eq!(state.attributes(), original.attributes());
    // La base garde les dates à la microseconde près.
    let delta = original
        .last_changed()
        .duration_since(state.last_changed())
        .unwrap();
    assert!(delta < Duration::from_micros(1));
    assert!(events.try_recv().is_err(), "la restauration ne publie rien");
}

#[tokio::test]
async fn sql_injection_attempt_is_stored_as_plain_text() {
    let (core, store, recorder) = setup().await;
    let evil = "'); DROP TABLE states; --";
    core.states
        .set(id("sensor.texte"), evil, attrs(json!({"note": evil})))
        .unwrap();
    recorder.stop().await;

    let states = store.load_states().await.unwrap();
    assert_eq!(states[0].state, evil);
    assert_eq!(states[0].attributes["note"], evil);
}

#[tokio::test]
async fn history_limit_is_respected() {
    let (core, store, recorder) = setup().await;
    let sensor = id("sensor.compteur");
    for value in 0..20 {
        core.states
            .set(sensor.clone(), value.to_string(), Attributes::new())
            .unwrap();
    }
    recorder.stop().await;

    let history = store.history(&sensor, 5).await.unwrap();
    assert_eq!(history.len(), 5);
    assert_eq!(history[0].state.as_ref().unwrap().state, "19");
}

#[tokio::test]
async fn lagging_recorder_resyncs_all_states() {
    let (core, store, recorder) = setup().await;
    // Le test tourne sur un seul fil : l'enregistreur ne peut rien écrire tant
    // qu'on ne fait pas de `.await`. On publie donc plus d'événements que le
    // bus ne peut en garder (1024) pour le forcer à en perdre.
    let count = ha_core::DEFAULT_BUS_CAPACITY + 100;
    for n in 0..count {
        core.states
            .set(id(&format!("sensor.s{n}")), "on", Attributes::new())
            .unwrap();
    }
    recorder.stop().await;

    // L'historique des événements perdus manque, mais tous les états sont là.
    assert_eq!(store.load_states().await.unwrap().len(), count);
}

#[tokio::test]
async fn purge_removes_only_old_history() {
    let (core, store, recorder) = setup().await;
    let light = id("light.salon");
    core.states
        .set(light.clone(), "on", Attributes::new())
        .unwrap();
    recorder.stop().await;

    let yesterday = SystemTime::now() - Duration::from_hours(24);
    assert_eq!(store.purge_history(yesterday).await.unwrap(), 0);

    let in_one_minute = SystemTime::now() + Duration::from_mins(1);
    assert_eq!(store.purge_history(in_one_minute).await.unwrap(), 1);
    assert!(store.history(&light, 10).await.unwrap().is_empty());
    // Le dernier état connu, lui, est conservé.
    assert_eq!(store.load_states().await.unwrap().len(), 1);
}

#[tokio::test]
async fn file_database_survives_a_restart() {
    let db = TempDb::new("restart");
    {
        let store = Store::open(&db.0).await.unwrap();
        let core = Arc::new(Core::new());
        let recorder = Recorder::start(store.clone(), Arc::clone(&core));
        core.states
            .set(id("light.salon"), "on", Attributes::new())
            .unwrap();
        recorder.stop().await;
        store.close().await;
    }

    let store = Store::open(&db.0).await.unwrap();
    let core = Core::new();
    assert_eq!(store.restore_into(&core.states).await.unwrap(), 1);
    assert_eq!(core.states.get(&id("light.salon")).unwrap().state(), "on");
    store.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn database_file_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let db = TempDb::new("private");
    let store = Store::open(&db.0).await.unwrap();
    let mode = std::fs::metadata(&db.0).unwrap().permissions().mode();
    store.close().await;

    assert_eq!(
        mode & 0o777,
        0o600,
        "seul le propriétaire doit pouvoir lire la base"
    );
}
