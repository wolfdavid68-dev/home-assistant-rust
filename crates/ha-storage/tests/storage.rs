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
async fn the_owning_integration_survives_a_restart() {
    let (core, store, recorder) = setup().await;
    let lamp = id("light.salon");
    core.states
        .set_by("hue", lamp.clone(), "on", attrs(json!({})))
        .unwrap();
    core.states
        .set(id("sensor.libre"), "1", attrs(json!({})))
        .unwrap();
    recorder.stop().await;

    let restarted = Core::new();
    store.restore_into(&restarted.states).await.unwrap();
    assert_eq!(restarted.states.owner_of(&lamp).as_deref(), Some("hue"));
    assert_eq!(restarted.states.owned_by("hue"), vec![lamp]);
    assert_eq!(restarted.states.owner_of(&id("sensor.libre")), None);
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

/// Ajoute une ligne d'historique à une date précise (en secondes).
async fn history_row(store: &Store, entity: &str, state: Option<&str>, at_seconds: u64) {
    let at =
        ha_storage::to_micros(std::time::UNIX_EPOCH + Duration::from_secs(at_seconds)).unwrap();
    sqlx::query(
        "INSERT INTO state_history (entity_id, state, attributes, last_changed, last_updated, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
    )
    .bind(entity)
    .bind(state)
    .bind(state.map(|_| "{}"))
    .bind(at)
    .execute(store.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn history_series_summarises_numbers_and_changes() {
    let store = Store::open_in_memory().await.unwrap();
    let base = 1_700_000_000;
    let time = |s: u64| std::time::UNIX_EPOCH + Duration::from_secs(base + s);

    // Température : deux mesures par tranche de 100 s, et une valeur non
    // numérique qui ne doit pas devenir un zéro.
    for (offset, value) in [
        (0, "20.0"),
        (50, "22.0"),
        (100, "unavailable"),
        (150, "25.5"),
        (160, "24.5"),
    ] {
        history_row(&store, "sensor.temperature", Some(value), base + offset).await;
    }
    let series = store
        .history_series(&id("sensor.temperature"), time(0), time(200), 2)
        .await
        .unwrap();
    let means: Vec<f64> = series.numeric.iter().map(|p| p.mean).collect();
    assert_eq!(means, [21.0, 25.0]);
    assert_eq!((series.numeric[1].min, series.numeric[1].max), (24.5, 25.5));

    // Porte : la valeur d'avant la période, puis seulement les vrais changements.
    for (offset, value) in [
        (0, Some("off")),
        (10, Some("on")),
        (20, Some("on")),
        (30, Some("off")),
        (40, None),
    ] {
        history_row(&store, "binary_sensor.porte", value, base + 1_000 + offset).await;
    }
    let series = store
        .history_series(&id("binary_sensor.porte"), time(1_005), time(2_000), 10)
        .await
        .unwrap();
    let changes: Vec<Option<&str>> = series.changes.iter().map(|c| c.state.as_deref()).collect();
    assert_eq!(changes, [Some("off"), Some("on"), Some("off"), None]);
    assert_eq!(
        series.changes[0].at,
        time(1_005),
        "valeur au début de la période"
    );
    assert_eq!(series.numeric, Vec::new(), "pas une valeur numérique");

    // Période vide ou à l'envers.
    let empty = store
        .history_series(&id("binary_sensor.porte"), time(2_000), time(1_000), 10)
        .await
        .unwrap();
    assert_eq!(empty, ha_storage::HistorySeries::default());
}

#[tokio::test]
async fn logbook_entries_are_filtered_cleaned_and_purged() {
    let store = Store::open_in_memory().await.unwrap();
    let door = id("binary_sensor.porte");
    let entry = |entity_id, name, message, cause| ha_storage::NewLogbookEntry {
        entity_id,
        name,
        message,
        cause,
    };
    store
        .add_logbook(entry(Some(&door), "Porte", "s'est ouverte", None))
        .await
        .unwrap();
    store
        .add_logbook(entry(
            None,
            "Bonne nuit",
            "automatisation lancée",
            Some("déclencheur : il est 22:30"),
        ))
        .await
        .unwrap();
    // Un nom piégé : caractères de contrôle remplacés, et l'injection SQL
    // reste du texte.
    store
        .add_logbook(entry(
            Some(&door),
            "Porte\n'); DROP TABLE logbook; --",
            "s'est fermée",
            Some("par David"),
        ))
        .await
        .unwrap();

    let now = SystemTime::now() + Duration::from_secs(1);
    let all = store
        .logbook(SystemTime::UNIX_EPOCH, now, None, 100)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].message, "s'est fermée", "le plus récent d'abord");
    assert_eq!(all[0].name, "Porte\u{fffd}'); DROP TABLE logbook; --");
    assert_eq!(all[0].cause.as_deref(), Some("par David"));
    assert_eq!(all[1].entity_id, None);

    let door_only = store
        .logbook(SystemTime::UNIX_EPOCH, now, Some(&door), 100)
        .await
        .unwrap();
    assert_eq!(door_only.len(), 2);
    let limited = store
        .logbook(SystemTime::UNIX_EPOCH, now, None, 1)
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);
    let future = store
        .logbook(now, now + Duration::from_mins(1), None, 100)
        .await
        .unwrap();
    assert_eq!(future, Vec::new());

    assert_eq!(
        store.purge_logbook(SystemTime::UNIX_EPOCH).await.unwrap(),
        0
    );
    assert_eq!(store.purge_logbook(now).await.unwrap(), 3);
}

/// Une ligne d'historique à une date précise (le recorder, lui, date à
/// « maintenant »).
async fn history_at(store: &Store, entity: &str, at: jiff::Timestamp, state: &str) {
    sqlx::query(
        "INSERT INTO state_history (entity_id, state, attributes, last_changed, last_updated, recorded_at)
         VALUES (?1, ?2, '{}', ?3, ?3, ?3)",
    )
    .bind(entity)
    .bind(state)
    .bind(at.as_microsecond())
    .execute(store.pool())
    .await
    .unwrap();
}

async fn current(store: &Store, entity: &str, attributes: serde_json::Value) {
    sqlx::query(
        "INSERT INTO states (entity_id, state, attributes, last_changed, last_updated)
         VALUES (?1, '0', ?2, 0, 0)",
    )
    .bind(entity)
    .bind(attributes.to_string())
    .execute(store.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn statistics_are_compiled_per_hour_and_grouped_per_local_day() {
    use ha_storage::Period;
    let store = Store::open_in_memory().await.unwrap();
    current(
        &store,
        "sensor.temperature",
        json!({"unit_of_measurement": "°C"}),
    )
    .await;
    current(
        &store,
        "sensor.compteur",
        json!({"unit_of_measurement": "kWh", "device_class": "energy"}),
    )
    .await;
    // Sans unité : ignoré.
    current(&store, "sensor.mode", json!({})).await;

    // Paris : 21 h UTC = 23 h, 22 h UTC = minuit le lendemain (heure d'été : +2 h).
    let at = |time: &str| time.parse::<jiff::Timestamp>().unwrap();
    history_at(
        &store,
        "sensor.temperature",
        at("2026-10-14T20:30:00Z"),
        "18",
    )
    .await;
    history_at(
        &store,
        "sensor.temperature",
        at("2026-10-14T21:00:00Z"),
        "20",
    )
    .await;
    history_at(
        &store,
        "sensor.temperature",
        at("2026-10-14T21:50:00Z"),
        "26",
    )
    .await;
    history_at(
        &store,
        "sensor.temperature",
        at("2026-10-14T22:30:00Z"),
        "unavailable",
    )
    .await;
    history_at(&store, "sensor.compteur", at("2026-10-14T20:59:00Z"), "100").await;
    history_at(
        &store,
        "sensor.compteur",
        at("2026-10-14T21:30:00Z"),
        "102.5",
    )
    .await;
    history_at(&store, "sensor.compteur", at("2026-10-14T22:10:00Z"), "0.5").await;
    history_at(&store, "sensor.mode", at("2026-10-14T21:10:00Z"), "12").await;

    let oldest = SystemTime::from(at("2026-10-14T21:00:00Z"));
    let now = SystemTime::from(at("2026-10-14T23:00:05Z"));
    assert_eq!(store.catch_up_statistics(oldest, now).await.unwrap(), 2);
    // Une deuxième fois : rien à refaire.
    assert_eq!(store.catch_up_statistics(oldest, now).await.unwrap(), 0);

    let zone = jiff::tz::TimeZone::get("Europe/Paris").unwrap();
    let id = |e: &str| e.parse::<EntityId>().unwrap();
    let all = |entity: &str, period| {
        let store = store.clone();
        let zone = zone.clone();
        let entity = id(entity);
        async move {
            store
                .statistics(&entity, SystemTime::UNIX_EPOCH, now, period, &zone)
                .await
                .unwrap()
        }
    };

    let hours = all("sensor.temperature", Period::Hour).await;
    assert_eq!(hours.len(), 2);
    // 21 h : 20 °C pendant 50 min, 26 °C pendant 10 min → 21 °C.
    assert!((hours[0].mean - 21.0).abs() < 1e-9, "{}", hours[0].mean);
    assert_eq!((hours[0].min, hours[0].max), (20.0, 26.0));
    // 22 h : 26 °C pendant 30 min, puis indisponible.
    assert!((hours[1].mean - 26.0).abs() < 1e-9);

    // Par jour (Paris) : 21 h UTC est le 14, 22 h UTC est déjà le 15.
    let days = all("sensor.temperature", Period::Day).await;
    assert_eq!(days.len(), 2);
    assert_eq!(
        jiff::Timestamp::try_from(days[1].start).unwrap(),
        at("2026-10-14T22:00:00Z"),
        "minuit à Paris"
    );

    // Compteur : 2,5 kWh puis 0,5 (après une remise à zéro), 3 kWh au mois.
    let counter = all("sensor.compteur", Period::Hour).await;
    assert_eq!(
        counter.iter().map(|p| p.change).collect::<Vec<_>>(),
        [Some(2.5), Some(0.5)]
    );
    let month = all("sensor.compteur", Period::Month).await;
    assert_eq!(month.len(), 1);
    assert_eq!(month[0].change, Some(3.0));

    assert_eq!(all("sensor.mode", Period::Hour).await, Vec::new());
}
