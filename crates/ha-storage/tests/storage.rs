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
use ha_storage::{AuditCheck, AuditRecord, Recorder, Store};
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
        let _ = std::fs::remove_file(self.0.with_extension("key"));
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

    assert_eq!(
        store.load_states().await.unwrap(),
        Vec::<ha_storage::StoredState>::new()
    );
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
    assert_eq!(
        store.history(&light, 10).await.unwrap(),
        Vec::<ha_storage::HistoryEntry>::new()
    );
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

#[tokio::test]
async fn secrets_survive_a_restart_but_need_the_key_file() {
    let db = TempDb::new("secrets");
    let sealed = {
        let store = Store::open(&db.0).await.unwrap();
        let sealed = store.seal("somfy_box.token", "jeton-très-secret").unwrap();
        store.close().await;
        sealed
    };
    assert!(
        db.0.with_extension("key").exists(),
        "clé dans un fichier séparé"
    );

    // Même base, même clé : le secret se relit.
    let store = Store::open(&db.0).await.unwrap();
    assert_eq!(
        store.unseal("somfy_box.token", &sealed).unwrap().as_str(),
        "jeton-très-secret"
    );
    store.close().await;

    // La base sans sa clé (une sauvegarde volée, par exemple) : illisible.
    std::fs::remove_file(db.0.with_extension("key")).unwrap();
    let store = Store::open(&db.0).await.unwrap();
    assert!(store.unseal("somfy_box.token", &sealed).is_err());
    store.close().await;
}

fn record<'a>(actor: &'a str, action: &'a str, target: Option<&'a str>) -> AuditRecord<'a> {
    AuditRecord {
        actor,
        action,
        target,
        detail: None,
        source: "192.168.1.10",
    }
}

#[tokio::test]
async fn audit_log_is_readable_and_tamper_evident() {
    let store = Store::open_in_memory().await.unwrap();
    assert_eq!(
        store.verify_audit().await.unwrap(),
        AuditCheck::Intact { entries: 0 }
    );
    store.audit(record("david", "login", None)).await.unwrap();
    store
        .audit(record("david", "service", Some("cover.volet_salon")))
        .await
        .unwrap();
    store.audit(record("david", "logout", None)).await.unwrap();

    let log = store.audit_log(10).await.unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!(log[0].action, "logout", "le plus récent d'abord");
    assert_eq!(log[1].target.as_deref(), Some("cover.volet_salon"));
    assert_eq!(
        store.verify_audit().await.unwrap(),
        AuditCheck::Intact { entries: 3 }
    );

    // Quelqu'un modifie une ligne directement dans la base…
    sqlx::query("UPDATE audit_log SET actor = 'quelquun' WHERE id = 2")
        .execute(store.pool())
        .await
        .unwrap();
    assert!(matches!(
        store.verify_audit().await.unwrap(),
        AuditCheck::Broken { id: 2, .. }
    ));
}

#[tokio::test]
async fn deleting_a_line_in_the_middle_is_detected_but_purging_is_not_an_error() {
    let store = Store::open_in_memory().await.unwrap();
    for action in ["a", "b", "c", "d"] {
        store.audit(record("david", action, None)).await.unwrap();
    }
    // La purge des lignes anciennes garde une chaîne vérifiable.
    sqlx::query("DELETE FROM audit_log WHERE id = 1")
        .execute(store.pool())
        .await
        .unwrap();
    assert_eq!(
        store.verify_audit().await.unwrap(),
        AuditCheck::Intact { entries: 3 }
    );
    // Une ligne supprimée au milieu : détecté.
    sqlx::query("DELETE FROM audit_log WHERE id = 3")
        .execute(store.pool())
        .await
        .unwrap();
    assert!(matches!(
        store.verify_audit().await.unwrap(),
        AuditCheck::Broken { id: 4, .. }
    ));
}

#[tokio::test]
async fn audit_lines_are_signed_with_the_key_file() {
    let db = TempDb::new("audit-key");
    let store = Store::open(&db.0).await.unwrap();
    store.audit(record("david", "login", None)).await.unwrap();
    store.close().await;

    // Sans la bonne clé, impossible de produire des signatures valides :
    // une ligne ajoutée par quelqu'un qui n'a que la base est détectée.
    std::fs::remove_file(db.0.with_extension("key")).unwrap();
    let store = Store::open(&db.0).await.unwrap();
    store.audit(record("intrus", "login", None)).await.unwrap();
    assert!(matches!(
        store.verify_audit().await.unwrap(),
        AuditCheck::Broken { id: 1, .. }
    ));
    store.close().await;
}

#[tokio::test]
async fn concurrent_audit_writes_keep_the_chain_intact() {
    let db = TempDb::new("audit-concurrent");
    let store = Store::open(&db.0).await.unwrap();
    // Deux « processus » (deux connexions indépendantes à la même base).
    let other = Store::open(&db.0).await.unwrap();
    let mut tasks = Vec::new();
    for i in 0..20 {
        let store = if i % 2 == 0 {
            store.clone()
        } else {
            other.clone()
        };
        tasks.push(tokio::spawn(async move {
            store
                .audit(record("david", "service", Some(&format!("light.l{i}"))))
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        store.verify_audit().await.unwrap(),
        AuditCheck::Intact { entries: 20 }
    );
    store.close().await;
    other.close().await;
}

#[tokio::test]
async fn old_audit_lines_are_purged() {
    let store = Store::open_in_memory().await.unwrap();
    store.audit(record("david", "login", None)).await.unwrap();
    let future = SystemTime::now() + Duration::from_mins(1);
    assert_eq!(store.purge_audit(future).await.unwrap(), 1);
    assert_eq!(store.audit_log(10).await.unwrap(), Vec::new());
}
