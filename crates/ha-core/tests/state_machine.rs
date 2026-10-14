//! Tests du noyau, vu de l'extérieur (seulement l'API publique).

// Dans un test, un `unwrap` qui échoue fait simplement échouer le test : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use ha_core::{Attributes, Core, CoreError, EntityId, Event, MAX_STATE_LEN};
use serde_json::json;
use tokio::sync::broadcast::error::TryRecvError;

fn id(value: &str) -> EntityId {
    value.parse().unwrap()
}

fn attrs(value: serde_json::Value) -> Attributes {
    value.as_object().unwrap().clone()
}

/// Récupère le prochain événement `StateChanged` reçu.
fn next_change(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<Event>>,
) -> (Option<String>, Option<String>) {
    let event = rx.try_recv().expect("un événement était attendu");
    let Event::StateChanged {
        old_state,
        new_state,
        ..
    } = event.as_ref();
    (
        old_state.as_ref().map(|s| s.state().to_owned()),
        new_state.as_ref().map(|s| s.state().to_owned()),
    )
}

#[test]
fn creating_an_entity_publishes_an_event() {
    let core = Core::new();
    let mut rx = core.bus.subscribe();

    let changed = core
        .states
        .set(id("light.salon"), "on", Attributes::new())
        .unwrap();

    assert!(changed);
    assert_eq!(next_change(&mut rx), (None, Some("on".into())));
    assert_eq!(core.states.get(&id("light.salon")).unwrap().state(), "on");
}

#[test]
fn identical_update_publishes_nothing() {
    let core = Core::new();
    core.states
        .set(id("light.salon"), "on", attrs(json!({"brightness": 200})))
        .unwrap();
    let mut rx = core.bus.subscribe();

    let changed = core
        .states
        .set(id("light.salon"), "on", attrs(json!({"brightness": 200})))
        .unwrap();

    assert!(!changed);
    assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
}

#[test]
fn attribute_change_keeps_last_changed() {
    let core = Core::new();
    let light = id("light.salon");
    core.states
        .set(light.clone(), "on", attrs(json!({"brightness": 100})))
        .unwrap();
    let before = core.states.get(&light).unwrap();

    core.states
        .set(light.clone(), "on", attrs(json!({"brightness": 200})))
        .unwrap();
    let after = core.states.get(&light).unwrap();

    assert_eq!(after.last_changed(), before.last_changed());
    assert!(after.last_updated() >= before.last_updated());
    assert_eq!(after.attributes()["brightness"], 200);
}

#[test]
fn value_change_reports_old_and_new_state() {
    let core = Core::new();
    let light = id("light.salon");
    core.states
        .set(light.clone(), "on", Attributes::new())
        .unwrap();
    let mut rx = core.bus.subscribe();

    core.states.set(light, "off", Attributes::new()).unwrap();

    assert_eq!(
        next_change(&mut rx),
        (Some("on".into()), Some("off".into()))
    );
}

#[test]
fn removing_an_entity_publishes_an_event() {
    let core = Core::new();
    let light = id("light.salon");
    core.states
        .set(light.clone(), "on", Attributes::new())
        .unwrap();
    let mut rx = core.bus.subscribe();

    assert!(core.states.remove(&light));
    assert_eq!(next_change(&mut rx), (Some("on".into()), None));
    assert!(core.states.get(&light).is_none());

    // Supprimer une entité qui n'existe plus ne fait rien.
    assert!(!core.states.remove(&light));
    assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
}

#[test]
fn too_long_state_is_rejected() {
    let core = Core::new();
    let too_long = "x".repeat(MAX_STATE_LEN + 1);

    let result = core
        .states
        .set(id("sensor.test"), too_long, Attributes::new());

    assert!(matches!(result, Err(CoreError::StateTooLong { .. })));
    assert!(core.states.get(&id("sensor.test")).is_none());
}

#[test]
fn all_returns_states_sorted_by_id() {
    let core = Core::new();
    for entity in ["switch.b", "light.z", "light.a"] {
        core.states
            .set(id(entity), "on", Attributes::new())
            .unwrap();
    }

    let ids: Vec<String> = core
        .states
        .all()
        .iter()
        .map(|s| s.entity_id().to_string())
        .collect();

    assert_eq!(ids, ["light.a", "light.z", "switch.b"]);
}

#[tokio::test]
async fn every_subscriber_receives_every_event() {
    let core = Arc::new(Core::new());
    let mut first = core.bus.subscribe();
    let mut second = core.bus.subscribe();

    // Une autre tâche modifie l'état pendant que les abonnés attendent.
    let writer = Arc::clone(&core);
    tokio::spawn(async move {
        writer
            .states
            .set(id("light.salon"), "on", Attributes::new())
            .unwrap();
    });

    for rx in [&mut first, &mut second] {
        let event = rx.recv().await.unwrap();
        let Event::StateChanged { entity_id, .. } = event.as_ref();
        assert_eq!(entity_id.as_str(), "light.salon");
    }
}
