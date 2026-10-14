//! Tests avec un **faux serveur Open-Meteo** : il enregistre les requêtes
//! et répond ce qu'on lui demande.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ha_core::Core;
use ha_sun::Location;
use ha_weather::{Config, Weather};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SAMPLE: &str = r#"{"current": {"temperature_2m": 31.4, "relative_humidity_2m": 35,
  "apparent_temperature": 33.0, "is_day": 1, "precipitation": 0.0, "weather_code": 0,
  "cloud_cover": 5, "pressure_msl": 1015.0, "wind_speed_10m": 12.0,
  "wind_direction_10m": 90, "wind_gusts_10m": 25.0},
 "daily": {"time": ["2026-07-01", "2026-07-02"], "weather_code": [0, 95],
  "temperature_2m_max": [33.0, 29.5], "temperature_2m_min": [19.0, 18.2],
  "precipitation_sum": [0.0, 15.0], "precipitation_probability_max": [0, 85]}}"#;

const LIMIT: &str = r#"{"error": true, "reason": "Daily API request limit exceeded."}"#;

/// (réponses à donner, requêtes reçues)
type Shared = (
    Arc<Mutex<Vec<(u16, &'static str)>>>,
    Arc<Mutex<Vec<String>>>,
);

async fn fake_server(answers: Vec<(u16, &'static str)>) -> (String, Shared) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/forecast", listener.local_addr().unwrap());
    let answers = Arc::new(Mutex::new(answers));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let shared = (Arc::clone(&answers), Arc::clone(&requests));
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            requests
                .lock()
                .unwrap()
                .push(head.lines().next().unwrap_or("").to_owned());
            let (status, body) = {
                let mut list = answers.lock().unwrap();
                if list.len() > 1 {
                    list.remove(0)
                } else {
                    list[0]
                }
            };
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    (url, shared)
}

async fn wait_for(core: &Core, entity: &str, state: &str) {
    for _ in 0..300 {
        if core
            .states
            .get(&entity.parse().unwrap())
            .is_some_and(|s| s.state() == state)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{entity} n'est jamais passé à {state}");
}

#[tokio::test]
async fn weather_and_sensors_are_published() {
    let (url, (_, requests)) = fake_server(vec![(200, SAMPLE)]).await;
    let location = Location::new(48.8566, 2.3522).unwrap();
    let config = Config::new(location, Some(&url)).unwrap();
    let core = Arc::new(Core::new());
    let weather = Weather::start(config, Arc::clone(&core)).unwrap();

    wait_for(&core, "weather.maison", "sunny").await;
    let state = core.states.get(&"weather.maison".parse().unwrap()).unwrap();
    let attributes = state.attributes();
    assert_eq!(attributes["temperature"], 31.4);
    assert_eq!(attributes["forecast"][1]["condition"], "lightning-rainy");
    assert_eq!(attributes["forecast"][1]["precipitation_probability"], 85.0);
    assert!(
        attributes["attribution"]
            .as_str()
            .unwrap()
            .contains("Open-Meteo")
    );

    let temperature = core
        .states
        .get(&"sensor.meteo_temperature".parse().unwrap())
        .unwrap();
    assert_eq!(temperature.state(), "31.4");
    assert_eq!(temperature.attributes()["unit_of_measurement"], "°C");
    for (entity, value) in [
        ("sensor.meteo_humidite", "35"),
        ("sensor.meteo_vent", "12.0"),
        ("sensor.meteo_pluie", "0.0"),
    ] {
        assert_eq!(
            core.states.get(&entity.parse().unwrap()).unwrap().state(),
            value,
            "{entity}"
        );
    }

    // La position envoyée est arrondie (48.8566 → 48.9).
    let request = requests.lock().unwrap()[0].clone();
    assert!(
        request.starts_with("GET /v1/forecast?latitude=48.9&longitude=2.4&"),
        "{request}"
    );
    weather.stop().await;
}

#[tokio::test]
async fn a_long_outage_marks_the_entities_unavailable() {
    // D'abord une bonne réponse, puis Open-Meteo refuse (limite atteinte).
    let (url, (_, requests)) = fake_server(vec![(200, SAMPLE), (429, LIMIT)]).await;
    let mut config = Config::new(Location::new(48.85, 2.35).unwrap(), Some(&url)).unwrap();
    config.interval = Duration::from_millis(50);
    config.stale_after = Duration::from_millis(300);
    let core = Arc::new(Core::new());
    let weather = Weather::start(config, Arc::clone(&core)).unwrap();

    wait_for(&core, "weather.maison", "sunny").await;
    // Pendant la panne, les dernières valeurs restent… puis deviennent
    // « indisponibles » : on ne fait pas croire qu'elles sont à jour.
    wait_for(&core, "weather.maison", "unavailable").await;
    wait_for(&core, "sensor.meteo_temperature", "unavailable").await;
    assert!(requests.lock().unwrap().len() >= 3, "on réessaie");
    weather.stop().await;
}
