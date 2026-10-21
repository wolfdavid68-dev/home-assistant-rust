//! De bout en bout, avec de fausses enceintes Sonos : chacune écoute sur sa
//! propre adresse locale (127.0.0.x, port 1400) et répond aux mêmes requêtes
//! UPnP qu'une vraie. Quand on lui demande de jouer une annonce, elle
//! télécharge le son comme le ferait une vraie enceinte.
//!
//! Linux seulement : il faut pouvoir écouter sur 127.0.0.2, 127.0.0.3…

#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::format_push_string,
    reason = "tests"
)]

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use ha_core::{Attributes, Core, EntityId, Origin};
use ha_sonos::announce::Timing;
use ha_sonos::{Config, Sonos, xml};
use ha_tts::{Engine, Tts};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Ce que sait une fausse enceinte.
#[derive(Debug, Clone, Default)]
struct Speaker {
    ip: String,
    transport: String,
    uri: String,
    uri_meta: String,
    track: u32,
    track_uri: String,
    track_meta: String,
    rel_time: String,
    volume: u8,
    muted: bool,
    /// Les ordres reçus, dans l'ordre.
    log: Vec<String>,
    /// La taille du dernier son téléchargé, et son début.
    downloaded: Option<(usize, Vec<u8>)>,
}

#[derive(Clone)]
struct Fake {
    me: Arc<Mutex<Speaker>>,
    topology: Arc<String>,
}

async fn soap(State(fake): State<Fake>, headers: HeaderMap, body: String) -> Response {
    let action = headers
        .get("soapaction")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim_matches('"').rsplit_once('#'))
        .map(|(_, action)| action.to_owned())
        .unwrap_or_default();
    let arg = |name: &str| xml::element_text(&body, name).unwrap_or_default();
    let mut me = fake.me.lock().unwrap();
    let fields: Vec<(&str, String)> = match action.as_str() {
        "GetZoneGroupState" => vec![("ZoneGroupState", fake.topology.to_string())],
        "GetTransportInfo" => vec![("CurrentTransportState", me.transport.clone())],
        "GetPositionInfo" => vec![
            ("Track", me.track.to_string()),
            ("TrackURI", me.track_uri.clone()),
            ("TrackMetaData", me.track_meta.clone()),
            ("RelTime", me.rel_time.clone()),
        ],
        "GetMediaInfo" => vec![
            ("CurrentURI", me.uri.clone()),
            ("CurrentURIMetaData", me.uri_meta.clone()),
        ],
        "GetVolume" => vec![("CurrentVolume", me.volume.to_string())],
        "GetMute" => vec![("CurrentMute", if me.muted { "1" } else { "0" }.to_owned())],
        "SetVolume" => {
            me.volume = arg("DesiredVolume").parse().unwrap();
            let line = format!("SetVolume {}", me.volume);
            me.log.push(line);
            vec![]
        }
        "SetMute" => {
            me.muted = arg("DesiredMute") == "1";
            let line = format!("SetMute {}", me.muted);
            me.log.push(line);
            vec![]
        }
        "SetAVTransportURI" => {
            me.uri = arg("CurrentURI");
            me.uri_meta = arg("CurrentURIMetaData");
            me.track_uri = if me.uri.starts_with("x-rincon-queue:") {
                "x-file:queue-track".into()
            } else {
                me.uri.clone()
            };
            me.transport = "STOPPED".into();
            let line = format!("SetAVTransportURI {}", me.uri);
            me.log.push(line);
            vec![]
        }
        "Seek" => {
            me.log
                .push(format!("Seek {} {}", arg("Unit"), arg("Target")));
            vec![]
        }
        "Play" => {
            me.log.push("Play".into());
            me.transport = "PLAYING".into();
            if me.uri.starts_with("http://") {
                // Comme une vraie enceinte : elle télécharge le son, le joue,
                // puis s'arrête.
                let url = me.uri.clone();
                let ip: IpAddr = me.ip.parse().unwrap();
                let speaker = Arc::clone(&fake.me);
                tokio::spawn(async move {
                    let bytes = download(ip, &url).await;
                    speaker.lock().unwrap().downloaded =
                        Some((bytes.len(), bytes[..4.min(bytes.len())].to_vec()));
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let mut speaker = speaker.lock().unwrap();
                    if speaker.track_uri == url {
                        speaker.transport = "STOPPED".into();
                    }
                });
            }
            vec![]
        }
        "Pause" => {
            if me.uri.starts_with("x-sonosapi-stream:") {
                me.log.push("Pause refusée".into());
                return (StatusCode::INTERNAL_SERVER_ERROR, "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>701</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>").into_response();
            }
            me.log.push("Pause".into());
            me.transport = "PAUSED_PLAYBACK".into();
            vec![]
        }
        "Stop" | "Next" | "Previous" => {
            me.log.push(action.clone());
            if action == "Stop" {
                me.transport = "STOPPED".into();
            }
            vec![]
        }
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "<errorCode>401</errorCode>",
            )
                .into_response();
        }
    };
    let mut answer = format!(
        "<?xml version=\"1.0\"?><s:Envelope><s:Body><u:{action}Response xmlns:u=\"urn:x\">"
    );
    for (name, value) in fields {
        answer.push_str(&format!("<{name}>{}</{name}>", xml::escape(&value)));
    }
    answer.push_str(&format!("</u:{action}Response></s:Body></s:Envelope>"));
    answer.into_response()
}

/// Télécharge comme une enceinte : depuis sa propre adresse (HTTP/1.0, tout
/// le corps jusqu'à la fermeture).
async fn download(from: IpAddr, url: &str) -> Vec<u8> {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::new(from, 0)).unwrap();
    let mut stream = socket.connect(host.parse().unwrap()).await.unwrap();
    stream
        .write_all(format!("GET /{path} HTTP/1.0\r\nHost: {host}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let head_end = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    assert!(
        response.starts_with(b"HTTP/1.0 200") || response.starts_with(b"HTTP/1.1 200"),
        "{}",
        String::from_utf8_lossy(&response[..head_end])
    );
    response[head_end + 4..].to_vec()
}

async fn start_speaker(speaker: Speaker, topology: Arc<String>) -> Arc<Mutex<Speaker>> {
    let address: SocketAddr = format!("{}:1400", speaker.ip).parse().unwrap();
    let me = Arc::new(Mutex::new(speaker));
    let app = Router::new().fallback(post(soap)).with_state(Fake {
        me: Arc::clone(&me),
        topology,
    });
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    me
}

fn didl(inner: &str) -> String {
    format!("<DIDL-Lite><item>{inner}</item></DIDL-Lite>")
}

fn member(uid: &str, ip: &str, name: &str) -> String {
    format!(
        "<ZoneGroupMember UUID=\"{uid}\" Location=\"http://{ip}:1400/xml/device_description.xml\" ZoneName=\"{name}\"/>"
    )
}

/// Un faux `wyoming-piper` qui rend une demi-seconde de son pour chaque
/// phrase, et note ce qu'on lui a demandé.
async fn fake_piper() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let noted = Arc::clone(&heard);
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = Vec::new();
            let mut byte = [0u8; 1];
            while stream.read_exact(&mut byte).await.is_ok() && byte[0] != b'\n' {
                line.push(byte[0]);
            }
            let request: Value = serde_json::from_slice(&line).unwrap();
            noted
                .lock()
                .unwrap()
                .push(request["data"]["text"].as_str().unwrap().to_owned());
            let format = json!({"rate": 22050, "width": 2, "channels": 1}).to_string();
            let payload = vec![0u8; 22_050];
            let mut reply = format!(
                "{{\"type\":\"audio-start\",\"data_length\":{}}}\n{format}",
                format.len()
            )
            .into_bytes();
            reply.extend(format!("{{\"type\":\"audio-chunk\",\"data_length\":{},\"payload_length\":{}}}\n{format}", format.len(), payload.len()).into_bytes());
            reply.extend(&payload);
            reply.extend(b"{\"type\":\"audio-stop\"}\n");
            stream.write_all(&reply).await.unwrap();
        }
    });
    (address, heard)
}

async fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..300 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("attendu en vain : {what}");
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn id(text: &str) -> EntityId {
    text.parse().unwrap()
}

fn data(value: Value) -> Attributes {
    match value {
        Value::Object(map) => map,
        _ => Attributes::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn speakers_play_announce_and_resume() {
    // Salon et Cuisine groupés (le salon coordonne), la chambre seule.
    let topology = Arc::new(format!(
        "<ZoneGroupState><ZoneGroups><ZoneGroup Coordinator=\"RINCON_S\">{}{}</ZoneGroup><ZoneGroup Coordinator=\"RINCON_C\">{}</ZoneGroup></ZoneGroups></ZoneGroupState>",
        member("RINCON_S", "127.0.0.2", "Salon"),
        member("RINCON_K", "127.0.0.3", "Cuisine"),
        member("RINCON_C", "127.0.0.4", "Chambre"),
    ));
    let salon = start_speaker(
        Speaker {
            ip: "127.0.0.2".into(),
            transport: "PLAYING".into(),
            uri: "x-rincon-queue:RINCON_S#0".into(),
            track: 3,
            track_uri: "x-file:queue-track".into(),
            track_meta: didl("<dc:title>Aline</dc:title><dc:creator>Christophe</dc:creator>"),
            rel_time: "0:01:23".into(),
            volume: 20,
            ..Speaker::default()
        },
        Arc::clone(&topology),
    )
    .await;
    let cuisine = start_speaker(
        Speaker {
            ip: "127.0.0.3".into(),
            volume: 30,
            muted: true,
            ..Speaker::default()
        },
        Arc::clone(&topology),
    )
    .await;
    let chambre = start_speaker(
        Speaker {
            ip: "127.0.0.4".into(),
            transport: "PLAYING".into(),
            uri: "x-sonosapi-stream:s15200?sid=254".into(),
            uri_meta: didl("<dc:title>FIP</dc:title>"),
            track_uri: "x-sonosapi-stream:s15200?sid=254".into(),
            track_meta: didl("<r:streamContent>Daft Punk - Get Lucky</r:streamContent>"),
            volume: 10,
            ..Speaker::default()
        },
        Arc::clone(&topology),
    )
    .await;

    let (piper, heard) = fake_piper().await;
    let core = Arc::new(Core::new());
    let mut config = Config::new(vec!["127.0.0.2:1400".into()]);
    config.audio_port = free_port();
    config.audio_address = Some("127.0.0.1".parse().unwrap());
    config.tts = Some(Tts::new(Engine::Wyoming {
        address: piper,
        voice: None,
    }));
    config.poll = Duration::from_millis(100);
    config.timing = Timing {
        poll: Duration::from_millis(20),
        start: Duration::from_secs(3),
        margin: Duration::from_secs(1),
    };
    let sonos = Sonos::start(config, Arc::clone(&core)).unwrap();

    // 1. Les enceintes apparaissent, avec ce qu'elles jouent.
    let state = |entity: &str| core.states.get(&id(entity));
    wait_until("le salon joue Aline", || {
        state("media_player.salon").is_some_and(|s| {
            s.state() == "playing" && s.attributes().get("media_title") == Some(&json!("Aline"))
        })
    })
    .await;
    let cuisine_state = state("media_player.cuisine").unwrap();
    assert_eq!(cuisine_state.state(), "playing", "la cuisine suit le salon");
    assert_eq!(cuisine_state.attributes()["volume_level"], json!(0.3));
    assert_eq!(cuisine_state.attributes()["is_volume_muted"], json!(true));
    assert_eq!(
        cuisine_state.attributes()["group_members"],
        json!(["media_player.salon"])
    );
    let chambre_state = state("media_player.chambre").unwrap();
    assert_eq!(chambre_state.attributes()["media_channel"], json!("FIP"));
    assert_eq!(
        chambre_state.attributes()["media_artist"],
        json!("Daft Punk")
    );
    assert_eq!(
        state("tts.annonces").unwrap().attributes()["enceintes"],
        json!([
            "media_player.chambre",
            "media_player.cuisine",
            "media_player.salon"
        ])
    );

    // 2. Une annonce sur la cuisine : c'est tout le groupe (le salon lit).
    let (_, reply) = core
        .call_service_tracked(
            Origin::User("david".into()),
            "tts",
            "speak",
            id("tts.annonces"),
            data(json!({"media_player_entity_id": "media_player.cuisine", "message": "Le repas est prêt", "volume": 45})),
        )
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), reply)
            .await
            .unwrap()
            .unwrap(),
        Ok(())
    );
    assert_eq!(heard.lock().unwrap().as_slice(), ["Le repas est prêt"]);
    // La musique reprend : même file, même morceau, même endroit.
    wait_until("la musique reprend", || {
        salon.lock().unwrap().log.last().map(String::as_str) == Some("Play")
            && salon.lock().unwrap().log.len() > 4
    })
    .await;
    let log = salon.lock().unwrap().log.clone();
    let announced = log
        .iter()
        .position(|l| l.starts_with("SetAVTransportURI http://127.0.0.1:"))
        .expect("annonce chargée");
    assert_eq!(log[announced - 1], "SetVolume 45");
    assert_eq!(
        &log[announced + 1..],
        [
            "Play",
            "SetAVTransportURI x-rincon-queue:RINCON_S#0",
            "Seek TRACK_NR 3",
            "Seek REL_TIME 0:01:23",
            "SetVolume 20",
            "Play",
        ]
    );
    let (size, start) = salon
        .lock()
        .unwrap()
        .downloaded
        .clone()
        .expect("son téléchargé");
    assert_eq!(start, b"RIFF");
    assert!(size > 44 + 44_100 * 4, "carillon et voix : {size} octets");
    // La cuisine : volume de l'annonce sans sourdine, puis comme avant.
    assert_eq!(
        cuisine.lock().unwrap().log,
        [
            "SetVolume 45",
            "SetMute false",
            "SetVolume 30",
            "SetMute true"
        ]
    );
    let last = state("tts.annonces").unwrap();
    assert_eq!(
        last.attributes()["dernier_message"],
        json!("Le repas est prêt")
    );

    // 3. Les ordres : pause sur la cuisine → au salon, qui coordonne ;
    //    volume sur la cuisine elle-même ; une radio s'arrête au lieu de
    //    se mettre en pause.
    for (service, entity, extra) in [
        ("media_pause", "media_player.cuisine", json!({})),
        (
            "volume_set",
            "media_player.cuisine",
            json!({"volume_level": 0.5}),
        ),
        ("media_pause", "media_player.chambre", json!({})),
    ] {
        let (_, reply) = core
            .call_service_tracked(
                Origin::User("david".into()),
                "media_player",
                service,
                id(entity),
                data(extra),
            )
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), reply)
                .await
                .unwrap()
                .unwrap(),
            Ok(()),
            "{service}"
        );
    }
    assert_eq!(
        salon.lock().unwrap().log.last().map(String::as_str),
        Some("Pause")
    );
    assert_eq!(
        cuisine.lock().unwrap().log.last().map(String::as_str),
        Some("SetVolume 50")
    );
    assert_eq!(chambre.lock().unwrap().log, ["Pause refusée", "Stop"]);
    wait_until("la chambre est arrêtée", || {
        state("media_player.chambre").is_some_and(|s| s.state() == "idle")
    })
    .await;

    // 4. Sans voix possible pour un mauvais texte : refusé avec la raison.
    let (_, reply) = core
        .call_service_tracked(
            Origin::System,
            "tts",
            "speak",
            id("tts.annonces"),
            data(json!({"media_player_entity_id": "media_player.inconnue", "message": "Bonjour"})),
        )
        .unwrap();
    let refused = tokio::time::timeout(Duration::from_secs(5), reply)
        .await
        .unwrap()
        .unwrap();
    assert!(refused.unwrap_err().contains("enceinte inconnue"));

    sonos.stop().await;
}
