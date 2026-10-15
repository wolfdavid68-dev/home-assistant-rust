//! Tests avec un **faux serveur Météo-France** : il enregistre les requêtes
//! et répond ce qu'on lui demande (une carte, ou un refus de la clé).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ha_core::Core;
use ha_weather::vigilance::{TODAY_ENTITY, TOMORROW_ENTITY, Vigilance, VigilanceConfig};
use jiff::{Timestamp, ToSpan};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const REPAIR: &str = "persistent_notification.repair_vigilance_cle";

/// Une carte : aujourd'hui orange (orages, en ce moment), demain jaune.
fn sample() -> String {
    let now = Timestamp::now();
    let start = now - 2.hours();
    let end = now + 10.hours();
    let after = end + 24.hours();
    format!(
        r#"{{"product": {{"update_time": "{now}", "periods": [
          {{"echeance": "J", "begin_validity_time": "{start}", "end_validity_time": "{end}",
            "timelaps": {{"domain_ids": [
              {{"domain_id": "29", "max_color_id": 3, "phenomenon_items": [
                {{"phenomenon_id": "3", "phenomenon_max_color_id": 3, "timelaps_items": [
                  {{"begin_time": "{start}", "end_time": "{end}", "color_id": 3}}]}},
                {{"phenomenon_id": "1", "phenomenon_max_color_id": 1, "timelaps_items": []}}]}}]}}}},
          {{"echeance": "J1", "begin_validity_time": "{end}", "end_validity_time": "{after}",
            "timelaps": {{"domain_ids": [
              {{"domain_id": "29", "max_color_id": 2, "phenomenon_items": [
                {{"phenomenon_id": "6", "phenomenon_max_color_id": 2, "timelaps_items": []}}]}}]}}}}
        ]}}}}"#
    )
}

type Answers = Arc<Mutex<Vec<(u16, String)>>>;
type Requests = Arc<Mutex<Vec<String>>>;

async fn fake_server(answers: Vec<(u16, String)>) -> (String, Answers, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/public/DPVigilance/v1/cartevigilance/encours",
        listener.local_addr().unwrap()
    );
    let answers = Arc::new(Mutex::new(answers));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (a, r) = (Arc::clone(&answers), Arc::clone(&requests));
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            r.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).to_string());
            let (status, body) = {
                let mut list = a.lock().unwrap();
                if list.len() > 1 {
                    list.remove(0)
                } else {
                    list[0].clone()
                }
            };
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    (url, answers, requests)
}

async fn wait_until(core: &Core, what: &str, check: impl Fn(&Core) -> bool) {
    for _ in 0..500 {
        if check(core) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("jamais arrivé : {what}");
}

fn state(core: &Core, entity: &str) -> Option<String> {
    core.states
        .get(&entity.parse().unwrap())
        .map(|s| s.state().to_owned())
}

fn start(url: &str, core: &Arc<Core>) -> Vigilance {
    let mut config = VigilanceConfig::new("29", "cle-de-test", Some(url)).unwrap();
    config.interval = Duration::from_millis(50);
    Vigilance::start(config, Arc::clone(core)).unwrap()
}

#[tokio::test]
async fn the_sensors_show_today_and_tomorrow() {
    let (url, _, requests) = fake_server(vec![(200, sample())]).await;
    let core = Arc::new(Core::new());
    let vigilance = start(&url, &core);
    wait_until(&core, "vigilance orange", |c| {
        state(c, TODAY_ENTITY).as_deref() == Some("Orange")
    })
    .await;
    assert_eq!(state(&core, TOMORROW_ENTITY).as_deref(), Some("Jaune"));
    let today = core.states.get(&TODAY_ENTITY.parse().unwrap()).unwrap();
    let a = today.attributes();
    assert_eq!(a["niveau_actuel"], "Orange");
    assert_eq!(a["Orages"], "Orange");
    assert_eq!(a["Vent violent"], "Vert");
    assert_eq!(a["departement"], "29");
    // Seuls les phénomènes au-dessus du vert sont détaillés.
    let active = a["phenomenes"].as_array().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0]["phenomene"], "Orages");
    assert_eq!(active[0]["plages"].as_array().unwrap().len(), 1);
    // La clé est envoyée dans l'en-tête `apikey`, jamais dans l'adresse.
    let first = requests.lock().unwrap()[0].clone();
    assert!(
        first.starts_with("GET /public/DPVigilance/v1/cartevigilance/encours HTTP/1.1"),
        "{first}"
    );
    assert!(
        first.to_ascii_lowercase().contains("apikey: cle-de-test"),
        "{first}"
    );
    vigilance.stop().await;
}

#[tokio::test]
async fn a_refused_key_raises_a_repair_alert_until_it_works_again() {
    let (url, answers, _) = fake_server(vec![(401, "{}".into())]).await;
    let core = Arc::new(Core::new());
    let vigilance = start(&url, &core);
    wait_until(&core, "alerte de réparation", |c| {
        state(c, REPAIR).is_some()
    })
    .await;
    let repair = core.states.get(&REPAIR.parse().unwrap()).unwrap();
    let message = repair.attributes()["message"].as_str().unwrap().to_owned();
    assert!(message.contains("portail-api.meteofrance.fr"), "{message}");
    assert!(!message.contains("cle-de-test"));
    // Sans aucune carte, les capteurs sont indisponibles, pas « vert ».
    assert_eq!(state(&core, TODAY_ENTITY).as_deref(), Some("unavailable"));

    // Nouvelle clé : la carte revient, l'alerte disparaît.
    *answers.lock().unwrap() = vec![(200, sample())];
    wait_until(&core, "alerte réglée", |c| {
        state(c, REPAIR).is_none() && state(c, TODAY_ENTITY).as_deref() == Some("Orange")
    })
    .await;
    vigilance.stop().await;
}

#[tokio::test]
async fn a_missing_department_is_not_taken_for_green() {
    let (url, _, _) = fake_server(vec![(200, sample().replace("\"29\"", "\"67\""))]).await;
    let core = Arc::new(Core::new());
    let vigilance = start(&url, &core);
    wait_until(&core, "indisponible", |c| {
        state(c, TODAY_ENTITY).as_deref() == Some("unavailable")
    })
    .await;
    // Ce n'est pas un problème de clé : pas d'alerte de réparation.
    assert!(state(&core, REPAIR).is_none());
    vigilance.stop().await;
}
