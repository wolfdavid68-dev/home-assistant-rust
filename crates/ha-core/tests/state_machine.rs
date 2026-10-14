//! Tests du noyau, vu de l'extérieur (seulement l'API publique).

// Dans un test, un `unwrap` qui échoue fait simplement échouer le test : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

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
    } = event.as_ref()
    else {
        panic!("StateChanged attendu, reçu {event:?}");
    };
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

#[test]
fn restore_keeps_dates_and_publishes_nothing() {
    let core = Core::new();
    let mut rx = core.bus.subscribe();
    let light = id("light.salon");
    let changed_at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let updated_at = changed_at + Duration::from_secs(60);

    let restored = core
        .states
        .restore(
            light.clone(),
            "on",
            attrs(json!({"brightness": 120})),
            changed_at,
            updated_at,
        )
        .unwrap();

    assert!(restored);
    assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
    let state = core.states.get(&light).unwrap();
    assert_eq!(state.state(), "on");
    assert_eq!(state.last_changed(), changed_at);
    assert_eq!(state.last_updated(), updated_at);
}

#[test]
fn restore_never_overwrites_a_live_state() {
    let core = Core::new();
    let light = id("light.salon");
    core.states
        .set(light.clone(), "off", Attributes::new())
        .unwrap();

    let restored = core
        .states
        .restore(
            light.clone(),
            "on",
            Attributes::new(),
            UNIX_EPOCH,
            UNIX_EPOCH,
        )
        .unwrap();

    assert!(!restored);
    assert_eq!(core.states.get(&light).unwrap().state(), "off");
}

#[test]
fn restore_rejects_too_long_state() {
    let core = Core::new();
    let too_long = "x".repeat(MAX_STATE_LEN + 1);

    let result = core.states.restore(
        id("sensor.test"),
        too_long,
        Attributes::new(),
        UNIX_EPOCH,
        UNIX_EPOCH,
    );

    assert!(matches!(result, Err(CoreError::StateTooLong { .. })));
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
        let Event::StateChanged { entity_id, .. } = event.as_ref() else {
            panic!("StateChanged attendu, reçu {event:?}");
        };
        assert_eq!(entity_id.as_str(), "light.salon");
    }
}

#[test]
fn call_service_is_published_on_the_bus() {
    let core = Core::new();
    let mut rx = core.bus.subscribe();

    core.call_service(
        "switch",
        "turn_on",
        id("switch.prise"),
        attrs(json!({"x": 1})),
    )
    .unwrap();

    let event = rx.try_recv().unwrap();
    let Event::CallService {
        domain,
        service,
        entity_id,
        data,
    } = event.as_ref()
    else {
        panic!("CallService attendu, reçu {event:?}");
    };
    assert_eq!(
        (domain.as_str(), service.as_str(), entity_id.as_str()),
        ("switch", "turn_on", "switch.prise")
    );
    assert_eq!(data["x"], 1);
}

#[test]
fn invalid_service_names_are_rejected() {
    let core = Core::new();
    let mut rx = core.bus.subscribe();
    for (domain, service) in [
        ("", "turn_on"),
        ("switch", ""),
        ("Switch", "turn_on"),
        ("switch", "turn on"),
        ("switch", "turn_on; rm"),
    ] {
        let result = core.call_service(domain, service, id("switch.prise"), Attributes::new());
        assert!(
            matches!(result, Err(CoreError::InvalidServiceName(_))),
            "{domain:?} {service:?}"
        );
    }
    assert_eq!(rx.try_recv().unwrap_err(), TryRecvError::Empty);
}
