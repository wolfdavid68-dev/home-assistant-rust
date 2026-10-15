//! L'agenda pour de vrai : une base en mémoire, le bus d'événements, et des
//! appels de service comme ceux du tableau de bord.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_agenda::Agenda;
use ha_core::{Attributes, Core, EntityId, Origin, State};
use ha_storage::Store;
use serde_json::{Value, json};

fn data(value: Value) -> Attributes {
    let Value::Object(map) = value else {
        panic!("un objet est attendu")
    };
    map
}

async fn setup() -> (Arc<Core>, Agenda, Store) {
    let core = Arc::new(Core::new());
    let store = Store::open_in_memory().await.unwrap();
    let agenda = Agenda::start(store.clone(), Arc::clone(&core)).await;
    (core, agenda, store)
}

/// Appelle un service puis attend que l'entité change (2 s au plus).
async fn call(core: &Core, domain: &str, service: &str, entity: &str, value: Value) -> State {
    let id: EntityId = entity.parse().unwrap();
    let before = core.states.get(&id);
    core.call_service_from(
        Origin::User("david".into()),
        domain,
        service,
        id.clone(),
        data(value),
    )
    .unwrap();
    for _ in 0..200 {
        let now = core.states.get(&id);
        if now != before {
            return now.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    before.unwrap()
}

#[tokio::test]
async fn a_shopping_list() {
    let (core, agenda, _) = setup().await;
    let list = core.states.get(&"todo.courses".parse().unwrap()).unwrap();
    assert_eq!(list.state(), "0");
    assert_eq!(list.attributes()["friendly_name"], "Liste de courses");

    call(
        &core,
        "todo",
        "add_item",
        "todo.courses",
        json!({"item": "Lait"}),
    )
    .await;
    let list = call(
        &core,
        "todo",
        "add_item",
        "todo.courses",
        json!({"item": "Pain", "due_date": "2026-10-20"}),
    )
    .await;
    assert_eq!(list.state(), "2");

    // Par son texte, comme le dit Home Assistant…
    let list = call(
        &core,
        "todo",
        "update_item",
        "todo.courses",
        json!({"item": "Lait", "status": "completed"}),
    )
    .await;
    assert_eq!(list.state(), "1");
    // … ou par son numéro.
    let uid = list.attributes()["items"][0]["uid"].clone();
    let list = call(
        &core,
        "todo",
        "update_item",
        "todo.courses",
        json!({"item": uid, "rename": "Pain complet"}),
    )
    .await;
    assert_eq!(list.attributes()["items"][0]["summary"], "Pain complet");

    let list = call(
        &core,
        "todo",
        "remove_completed_items",
        "todo.courses",
        json!({}),
    )
    .await;
    assert_eq!(list.attributes()["items"].as_array().unwrap().len(), 1);

    // Un texte refusé ne change rien.
    let refused = call(
        &core,
        "todo",
        "add_item",
        "todo.courses",
        json!({"item": "x".repeat(500)}),
    )
    .await;
    assert_eq!(refused.state(), "1");
    agenda.stop().await;
}

#[tokio::test]
async fn the_household_calendar() {
    let (core, agenda, store) = setup().await;
    let tomorrow = jiff::Zoned::now().date().tomorrow().unwrap();
    let calendar = call(
        &core,
        "calendar",
        "create_event",
        "calendar.maison",
        json!({
            "summary": "Dentiste",
            "start_date_time": format!("{tomorrow} 18:30"),
            "end_date_time": format!("{tomorrow} 19:00"),
        }),
    )
    .await;
    assert_eq!(calendar.state(), "off");
    assert_eq!(calendar.attributes()["message"], "Dentiste");
    let events = calendar.attributes()["events"].as_array().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["created_by"], "david");
    assert_eq!(events[0]["start"], format!("{tomorrow}T18:30:00"));

    // Un rendez-vous en cours : l'agenda passe à « on ».
    let now = jiff::Zoned::now().datetime();
    let calendar = call(
        &core,
        "calendar",
        "create_event",
        "calendar.maison",
        json!({
            "summary": "Réunion",
            "start_date_time": now.checked_sub(jiff::ToSpan::minutes(10)).unwrap().to_string(),
            "end_date_time": now.checked_add(jiff::ToSpan::minutes(50)).unwrap().to_string(),
        }),
    )
    .await;
    assert_eq!(calendar.state(), "on");
    assert_eq!(calendar.attributes()["message"], "Réunion");

    let uid = calendar.attributes()["events"][0]["uid"].clone();
    let calendar = call(
        &core,
        "calendar",
        "delete_event",
        "calendar.maison",
        json!({"uid": uid}),
    )
    .await;
    assert_eq!(calendar.state(), "off");

    // Fin avant le début : refusé, rien n'est écrit.
    call(
        &core,
        "calendar",
        "create_event",
        "calendar.maison",
        json!({"summary": "Absurde", "start_date_time": "2026-10-15 19:00", "end_date_time": "2026-10-15 18:00"}),
    )
    .await;
    let all = store
        .calendar_events(
            "calendar.maison",
            "2000-01-01T00:00:00",
            "2100-01-01T00:00:00",
            100,
        )
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    agenda.stop().await;
}
