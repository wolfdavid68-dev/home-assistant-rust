//! Le bilan, calculé depuis de vraies statistiques en base.

#![allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use ha_core::{Attributes, Core};
use ha_energy::{Kind, parse, report};
use ha_storage::Store;
use jiff::Timestamp;
use jiff::tz::TimeZone;

async fn hour(store: &Store, entity: &str, at: &str, change: f64) {
    let start = at.parse::<Timestamp>().unwrap().as_microsecond();
    sqlx_query(store, entity, start, change).await;
}

async fn sqlx_query(store: &Store, entity: &str, start: i64, change: f64) {
    // Paramètres liés, comme partout ailleurs.
    sqlx::query(
        "INSERT INTO statistics (entity_id, start, mean, min, max, last, change)
         VALUES (?1, ?2, 0, 0, 0, 0, ?3)",
    )
    .bind(entity)
    .bind(start)
    .bind(change)
    .execute(store.pool())
    .await
    .unwrap();
}

fn linky(core: &Core, key: &str, name: &str) {
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), name.into());
    attributes.insert("state_class".into(), "total_increasing".into());
    attributes.insert("unit_of_measurement".into(), "kWh".into());
    core.states
        .set(
            format!("sensor.linky_{key}").parse().unwrap(),
            "1",
            attributes,
        )
        .unwrap();
}

#[tokio::test]
async fn days_months_and_costs() {
    let store = Store::open_in_memory().await.unwrap();
    hour(
        &store,
        "sensor.linky_heures_creuses",
        "2026-09-20T01:00:00Z",
        10.0,
    )
    .await;
    hour(
        &store,
        "sensor.linky_heures_creuses",
        "2026-10-14T01:00:00Z",
        2.0,
    )
    .await;
    hour(
        &store,
        "sensor.linky_heures_creuses",
        "2026-10-14T02:00:00Z",
        1.5,
    )
    .await;
    hour(
        &store,
        "sensor.linky_heures_creuses",
        "2026-10-15T03:00:00Z",
        1.0,
    )
    .await;
    hour(
        &store,
        "sensor.linky_heures_pleines",
        "2026-10-15T09:00:00Z",
        3.0,
    )
    .await;
    hour(
        &store,
        "sensor.linky_production",
        "2026-10-15T10:00:00Z",
        2.0,
    )
    .await;
    // L'an dernier : pas compté dans l'année.
    hour(
        &store,
        "sensor.linky_heures_pleines",
        "2025-12-31T10:00:00Z",
        99.0,
    )
    .await;

    let core = Core::new();
    linky(&core, "energie", "Linky consommation");
    linky(&core, "heures_creuses", "Linky heures creuses");
    linky(&core, "heures_pleines", "Linky heures pleines");
    linky(&core, "production", "Linky production injectée");
    let now = "2026-10-15T12:00:00Z"
        .parse::<Timestamp>()
        .unwrap()
        .to_zoned(TimeZone::UTC);

    // Sans fichier : les index du Linky (pas le total, qui compterait double).
    let automatic = report(&store, &core.states, &parse("").unwrap(), 30, &now)
        .await
        .unwrap();
    assert!(!automatic.configured);
    let names: Vec<(&str, Kind)> = automatic
        .sources
        .iter()
        .map(|s| (s.name.as_str(), s.kind))
        .collect();
    assert_eq!(
        names,
        [
            ("Linky heures creuses", Kind::Consumption),
            ("Linky heures pleines", Kind::Consumption),
            ("Linky production injectée", Kind::Production),
        ]
    );
    let days: Vec<(&str, &[Option<f64>])> = automatic
        .days
        .iter()
        .map(|d| (d.date.as_str(), d.kwh.as_slice()))
        .collect();
    assert_eq!(
        days,
        [
            ("2026-09-20", &[Some(10.0), None, None][..]),
            ("2026-10-14", &[Some(3.5), None, None][..]),
            ("2026-10-15", &[Some(1.0), Some(3.0), Some(2.0)][..]),
        ]
    );
    assert_eq!(automatic.today.consumed_kwh, 4.0);
    assert_eq!(automatic.today.produced_kwh, 2.0);
    assert_eq!(automatic.today.cost, None);
    assert_eq!(automatic.month.consumed_kwh, 7.5);
    assert_eq!(automatic.year.consumed_kwh, 17.5);
    assert_eq!(automatic.months.len(), 2);

    // Avec les prix.
    let config = parse(
        "consumption:\n  - {entity: sensor.linky_heures_creuses, price: 0.2}\n  \
         - {entity: sensor.linky_heures_pleines, price: 0.3}\n\
         production:\n  - {entity: sensor.linky_production, price: 0.1}\nsubscription: 15\n",
    )
    .unwrap();
    let priced = report(&store, &core.states, &config, 7, &now)
        .await
        .unwrap();
    assert!(priced.configured);
    // Une semaine : le 20 septembre n'y est plus.
    assert_eq!(priced.days.len(), 2);
    let cents = |v: Option<f64>| v.map(|v| (v * 100.0).round());
    assert_eq!(cents(priced.today.cost), Some(110.0));
    assert_eq!(cents(priced.today.revenue), Some(20.0));
    assert_eq!(cents(priced.year.cost), Some(380.0));
    assert_eq!(priced.subscription, Some(15.0));
}
