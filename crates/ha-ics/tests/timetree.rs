//! TimeTree de bout en bout, avec un faux site TimeTree : connexion,
//! session expirée, événements par morceaux, mot de passe refusé.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use ha_core::{Core, EntityId};
use ha_ics::timetree::{Client, TimeTreeError};
use ha_storage::TimeTreeAccount;
use serde_json::{Value, json};
use zeroize::Zeroizing;

#[derive(Default)]
struct Site {
    logins: usize,
    /// La prochaine lecture répond 401 (session expirée).
    expire_next: bool,
    session: String,
}

type Shared = Arc<Mutex<Site>>;

async fn signin(State(site): State<Shared>, body: String) -> Response {
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["uuid"].as_str().unwrap().len(), 32);
    if body["uid"] != "maison@exemple.fr" || body["password"] != "bon mot de passe" {
        return (
            StatusCode::UNAUTHORIZED,
            json!({"error": {"code": -702}}).to_string(),
        )
            .into_response();
    }
    let mut site = site.lock().unwrap();
    site.logins += 1;
    site.session = format!("session{}", site.logins);
    (
        [(
            header::SET_COOKIE,
            format!("_session_id={}; path=/; HttpOnly", site.session),
        )],
        "{}",
    )
        .into_response()
}

fn allowed(site: &Shared, headers: &HeaderMap) -> bool {
    let mut site = site.lock().unwrap();
    if std::mem::take(&mut site.expire_next) {
        return false;
    }
    headers.get("x-timetreea").is_some()
        && headers.get(header::COOKIE).and_then(|c| c.to_str().ok())
            == Some(&format!("_session_id={}", site.session))
}

async fn calendars(State(site): State<Shared>, headers: HeaderMap) -> Response {
    if !allowed(&site, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    json!({"calendars": [
        {"id": 11, "name": "Travail"},
        {"id": 22, "name": "Famille Martin"},
        {"id": 33, "name": "Famille Martin", "deactivated_at": 1},
    ]})
    .to_string()
    .into_response()
}

async fn events(
    State(site): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !allowed(&site, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    assert_eq!(id, 22);
    // 2026-10-21 18:00, heure de Paris.
    let start = 1_792_533_600_000_i64 + 18 * 3_600_000;
    let page = if query.get("since").map(String::as_str) == Some("0") {
        json!({"chunk": true, "since": 500, "events": [
            {"uuid": "e1", "title": "Piscine", "start_at": start, "end_at": start + 3_600_000,
             "all_day": false, "start_timezone": "Europe/Paris", "type": 0, "category": 1,
             "recurrences": ["RRULE:FREQ=WEEKLY"]}
        ]})
    } else {
        json!({"chunk": false, "since": 600, "events": [
            {"uuid": "e2", "title": "Dentiste", "start_at": start + 86_400_000, "end_at": start + 86_400_000 + 1_800_000,
             "all_day": false, "start_timezone": "Europe/Paris", "type": 0, "category": 1}
        ]})
    };
    page.to_string().into_response()
}

async fn site() -> (String, Shared) {
    let shared: Shared = Arc::default();
    let app = Router::new()
        .route("/api/v1/auth/email/signin", put(signin))
        .route("/api/v1/calendars", get(calendars))
        .route("/api/v1/calendar/{id}/events/sync", get(events))
        .with_state(Arc::clone(&shared));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, shared)
}

fn account(password: &str) -> TimeTreeAccount {
    TimeTreeAccount {
        email: "maison@exemple.fr".into(),
        password: Zeroizing::new(password.into()),
    }
}

#[tokio::test]
async fn the_client_logs_in_and_reads_every_chunk() {
    let (base, shared) = site().await;
    let client = Client::new(
        ha_tls::web_client(false).unwrap(),
        &base,
        account("bon mot de passe"),
    );
    let found = client.find("famille martin").await.unwrap();
    assert_eq!(
        found.id, 22,
        "l'agenda actif, sans tenir compte des majuscules"
    );
    // La session expire : le client se reconnecte une fois, tout seul.
    shared.lock().unwrap().expire_next = true;
    let list = client.events(22).await.unwrap();
    assert_eq!(list.len(), 2, "les deux morceaux");
    assert_eq!(shared.lock().unwrap().logins, 2);
    let missing = client.find("Vacances").await.unwrap_err();
    assert_eq!(
        missing,
        TimeTreeError::NotFound("Vacances".into(), "Travail, Famille Martin".into())
    );

    let refused = Client::new(ha_tls::web_client(false).unwrap(), &base, account("faux"));
    assert_eq!(
        refused.calendars().await.unwrap_err(),
        TimeTreeError::Credentials
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timetree_calendar_becomes_a_house_calendar() {
    let (base, _) = site().await;
    let calendars = ha_ics::parse_config("- nom: Famille\n  timetree: Famille Martin\n").unwrap();
    let core = Arc::new(Core::new());
    let service = ha_ics::Calendars::start_with(
        calendars,
        Some(account("bon mot de passe")),
        &base,
        Arc::clone(&core),
    )
    .unwrap();
    let id: EntityId = "calendar.famille".parse().unwrap();
    let mut state = None;
    for _ in 0..200 {
        state = core.states.get(&id);
        if state.as_ref().is_some_and(|s| s.state() != "unavailable") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let state = state.expect("agenda publié");
    assert_ne!(state.state(), "unavailable", "{:?}", state.attributes());
    assert_eq!(state.attributes()["source"], "TimeTree");
    assert_eq!(state.attributes()["read_only"], true);
    let events = state.attributes()["events"].as_array().unwrap();
    let titles: Vec<&str> = events
        .iter()
        .filter_map(|e| e["summary"].as_str())
        .collect();
    assert!(titles.contains(&"Dentiste"), "{titles:?}");
    assert!(
        titles.iter().filter(|t| **t == "Piscine").count() >= 8,
        "une piscine par semaine : {titles:?}"
    );
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_shows_on_the_calendar() {
    let (base, _) = site().await;
    let calendars = ha_ics::parse_config("- nom: Famille\n  timetree: Famille Martin\n").unwrap();
    let core = Arc::new(Core::new());
    let service =
        ha_ics::Calendars::start_with(calendars, Some(account("faux")), &base, Arc::clone(&core))
            .unwrap();
    let id: EntityId = "calendar.famille".parse().unwrap();
    for _ in 0..200 {
        if core.states.get(&id).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let state = core.states.get(&id).unwrap();
    assert_eq!(state.state(), "unavailable");
    assert_eq!(
        state.attributes()["erreur"],
        "adresse e-mail ou mot de passe TimeTree refusé"
    );
    service.stop().await;

    // Sans compte associé : la raison, et comment faire.
    let calendars = ha_ics::parse_config("- nom: Famille\n  timetree: Famille Martin\n").unwrap();
    let core = Arc::new(Core::new());
    let service = ha_ics::Calendars::start_with(calendars, None, &base, Arc::clone(&core)).unwrap();
    for _ in 0..200 {
        if core.states.get(&id).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let state = core.states.get(&id).unwrap();
    assert!(
        state.attributes()["erreur"]
            .as_str()
            .unwrap()
            .contains("ha-server timetree pair")
    );
    service.stop().await;
}
