//! Le faux pont Hue : un petit serveur HTTP, **séparé** de l'API du tableau de
//! bord, qui ne connaît que les appareils de `alexa.yaml`.
//!
//! Sécurité : l'API Hue n'a pas de vraie authentification (un Echo ne sait
//! pas en faire). Les protections sont donc ailleurs :
//! - seules les adresses autorisées sont servies (par défaut : le réseau
//!   local uniquement ; mieux : l'adresse de tes Echo, `HA_ALEXA_ALLOWED`) ;
//! - seuls les appareils listés sont visibles et pilotables, et jamais un
//!   portail ou une porte de garage ;
//! - chaque ordre passe par le journal d'audit, comme depuis le tableau de bord ;
//! - corps de requête limités à 4 Kio, requêtes limitées à 5 secondes.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use ha_core::Core;
use ha_storage::{AuditRecord, Store};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tower_http::timeout::TimeoutLayer;

use crate::config::Device;
use crate::hue;

/// Combien de temps on « promet » à Alexa l'état qu'elle vient de demander.
///
/// Juste après un ordre, Alexa relit l'état de la lampe. Si l'appareil n'a
/// pas encore confirmé (un volet met du temps), elle dirait « l'appareil ne
/// répond pas ». Home Assistant fait pareil.
const PROMISE_LIFETIME: Duration = Duration::from_secs(5);
const MAX_BODY_SIZE: usize = 4 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Qui a le droit de parler au faux pont.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Allowed {
    /// Toute adresse du réseau local (privée), et la machine elle-même.
    LocalNetwork,
    /// Uniquement ces adresses (celles des Echo).
    Only(Vec<IpAddr>),
}

impl Allowed {
    #[must_use]
    pub fn permits(&self, ip: IpAddr) -> bool {
        // Une adresse IPv4 vue en IPv6 (`::ffff:192.168.1.10`) est ramenée à IPv4.
        let ip = ip.to_canonical();
        match self {
            Self::Only(list) => list.iter().any(|allowed| allowed.to_canonical() == ip),
            Self::LocalNetwork => match ip {
                IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
                IpAddr::V6(v6) => {
                    let first = v6.segments()[0];
                    v6.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
                }
            },
        }
    }
}

/// L'identité du faux pont, dérivée de son adresse : stable d'un démarrage
/// à l'autre.
#[derive(Debug, Clone)]
pub struct Identity {
    pub ip: Ipv4Addr,
    pub port: u16,
    /// Six octets, en hexadécimal : `001788` (préfixe Philips) + 3 octets.
    pub serial: String,
}

impl Identity {
    #[must_use]
    pub fn new(ip: Ipv4Addr, port: u16) -> Self {
        let hash = Sha256::digest(format!("ha-alexa {ip}").as_bytes());
        Self {
            ip,
            port,
            serial: format!("001788{:02x}{:02x}{:02x}", hash[0], hash[1], hash[2]),
        }
    }

    #[must_use]
    pub fn location(&self) -> String {
        format!("http://{}:{}/description.xml", self.ip, self.port)
    }

    #[must_use]
    pub fn bridge_id(&self) -> String {
        let serial = self.serial.to_uppercase();
        format!("{}FFFE{}", &serial[..6], &serial[6..])
    }

    #[must_use]
    pub fn uuid(&self) -> String {
        format!("2f402f80-da50-11e1-9b23-{}", self.serial)
    }

    fn mac(&self) -> String {
        self.serial
            .as_bytes()
            .chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).into_owned())
            .collect::<Vec<_>>()
            .join(":")
    }
}

/// Tout ce que le serveur partage entre les requêtes.
#[derive(Debug)]
pub struct Bridge {
    pub core: Arc<Core>,
    pub store: Store,
    /// Numéro de lampe → appareil.
    pub lights: BTreeMap<u32, Device>,
    pub allowed: Allowed,
    pub identity: Identity,
    promised: Mutex<HashMap<u32, (bool, u8, Instant)>>,
}

impl Bridge {
    #[must_use]
    pub fn new(
        core: Arc<Core>,
        store: Store,
        lights: BTreeMap<u32, Device>,
        allowed: Allowed,
        identity: Identity,
    ) -> Self {
        Self {
            core,
            store,
            lights,
            allowed,
            identity,
            promised: Mutex::default(),
        }
    }

    /// L'état promis à Alexa pour cette lampe, s'il est encore récent.
    fn promise(&self, number: u32) -> Option<(bool, u8)> {
        let promised = self.promised.lock().ok()?;
        let (on, bri, at) = promised.get(&number)?;
        (at.elapsed() < PROMISE_LIFETIME).then_some((*on, *bri))
    }

    fn remember(&self, number: u32, promised: (bool, u8)) {
        if let Ok(mut map) = self.promised.lock() {
            map.retain(|_, (_, _, at)| at.elapsed() < PROMISE_LIFETIME);
            map.insert(number, (promised.0, promised.1, Instant::now()));
        }
    }

    fn light(&self, number: u32, device: &Device) -> Option<Value> {
        let state = self.core.states.get(&device.entity_id);
        hue::is_allowed(&device.entity_id, state.as_ref())
            .then(|| hue::light_json(device, state.as_ref(), self.promise(number)))
    }

    fn lights_json(&self) -> Value {
        let mut lights = Map::new();
        for (number, device) in &self.lights {
            if let Some(light) = self.light(*number, device) {
                lights.insert(number.to_string(), light);
            }
        }
        Value::Object(lights)
    }

    fn config_json(&self) -> Value {
        json!({
            "name": "ha-server",
            "bridgeid": self.identity.bridge_id(),
            "mac": self.identity.mac(),
            "ipaddress": self.identity.ip.to_string(),
            "dhcp": true,
            "modelid": "BSB002",
            "swversion": "1935144020",
            "apiversion": "1.17.0",
            "linkbutton": true,
            "portalservices": false,
            "whitelist": {},
        })
    }
}

/// Les routes du faux pont.
pub fn router(bridge: Arc<Bridge>) -> Router {
    Router::new()
        .route("/description.xml", get(description))
        .route("/api", post(create_user))
        .route("/api/config", get(config))
        .route("/api/{user}", get(full_state))
        .route("/api/{user}/config", get(user_config))
        .route("/api/{user}/lights", get(lights))
        .route("/api/{user}/groups", get(groups))
        .route("/api/{user}/lights/{number}", get(light))
        .route("/api/{user}/lights/{number}/state", put(set_state))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(middleware::from_fn_with_state(Arc::clone(&bridge), guard))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .with_state(bridge)
}

/// Refuse les adresses non autorisées, **avant** toute autre chose.
async fn guard(
    State(bridge): State<Arc<Bridge>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if bridge.allowed.permits(client.ip()) {
        next.run(request).await
    } else {
        tracing::warn!(
            "Alexa : requête refusée depuis {} (adresse non autorisée)",
            client.ip()
        );
        StatusCode::FORBIDDEN.into_response()
    }
}

fn hue_json(value: &Value) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

async fn not_found(request: Request) -> Response {
    let address = request.uri().path().to_owned();
    hue_json(&hue::error_response(3, &address, "resource not available"))
}

/// La fiche UPnP du pont : c'est elle qu'un Echo lit après la découverte.
async fn description(State(bridge): State<Arc<Bridge>>) -> Response {
    let id = &bridge.identity;
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
<root xmlns="urn:schemas-upnp-org:device-1-0">
<specVersion><major>1</major><minor>0</minor></specVersion>
<URLBase>http://{ip}:{port}/</URLBase>
<device>
<deviceType>urn:schemas-upnp-org:device:Basic:1</deviceType>
<friendlyName>ha-server ({ip})</friendlyName>
<manufacturer>Royal Philips Electronics</manufacturer>
<manufacturerURL>http://www.philips.com</manufacturerURL>
<modelDescription>Philips hue Personal Wireless Lighting</modelDescription>
<modelName>Philips hue bridge 2015</modelName>
<modelNumber>BSB002</modelNumber>
<modelURL>http://www.meethue.com</modelURL>
<serialNumber>{serial}</serialNumber>
<UDN>uuid:{uuid}</UDN>
</device>
</root>
"#,
        ip = id.ip,
        port = id.port,
        serial = id.serial,
        uuid = id.uuid(),
    );
    ([(header::CONTENT_TYPE, "text/xml; charset=utf-8")], xml).into_response()
}

/// `POST /api` : un Echo « s'inscrit ». On accepte (pas de bouton à presser),
/// puisque seules les adresses autorisées arrivent jusqu'ici.
async fn create_user(
    State(bridge): State<Arc<Bridge>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
) -> Response {
    tracing::info!("Alexa : appareil inscrit depuis {}", client.ip());
    // Un nom fixe : il ne protège rien (n'importe quel nom est accepté),
    // c'est le filtre d'adresses qui protège.
    let hash = Sha256::digest(bridge.identity.serial.as_bytes());
    let username = hash.iter().take(16).fold(String::new(), |mut text, byte| {
        // Concept Rust : `write!` dans une `String` évite de créer une
        // petite chaîne par octet. Écrire dans une `String` ne peut pas échouer.
        let _ = write!(text, "{byte:02x}");
        text
    });
    hue_json(&json!([{ "success": { "username": username } }]))
}

async fn config(State(bridge): State<Arc<Bridge>>) -> Response {
    hue_json(&bridge.config_json())
}

async fn user_config(State(bridge): State<Arc<Bridge>>, Path(_user): Path<String>) -> Response {
    hue_json(&bridge.config_json())
}

async fn full_state(State(bridge): State<Arc<Bridge>>, Path(_user): Path<String>) -> Response {
    hue_json(&json!({
        "lights": bridge.lights_json(),
        "groups": {},
        "config": bridge.config_json(),
    }))
}

async fn lights(State(bridge): State<Arc<Bridge>>, Path(_user): Path<String>) -> Response {
    hue_json(&bridge.lights_json())
}

async fn groups(Path(_user): Path<String>) -> Response {
    hue_json(&json!({}))
}

/// Trouve une lampe par son numéro (dans l'adresse).
fn find(bridge: &Bridge, number: &str) -> Option<(u32, Device)> {
    let number: u32 = number.parse().ok()?;
    bridge
        .lights
        .get(&number)
        .map(|device| (number, device.clone()))
}

async fn light(
    State(bridge): State<Arc<Bridge>>,
    Path((_user, number)): Path<(String, String)>,
) -> Response {
    let address = format!("/lights/{number}");
    match find(&bridge, &number).and_then(|(n, device)| bridge.light(n, &device)) {
        Some(light) => hue_json(&light),
        None => hue_json(&hue::error_response(3, &address, "resource not available")),
    }
}

/// `PUT /api/{user}/lights/{n}/state` : l'ordre d'Alexa.
async fn set_state(
    State(bridge): State<Arc<Bridge>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    Path((_user, number)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let address = format!("/lights/{number}/state");
    let Some((number, device)) = find(&bridge, &number) else {
        return hue_json(&hue::error_response(3, &address, "resource not available"));
    };
    let entity_id = &device.entity_id;
    let state = bridge.core.states.get(entity_id);
    if !hue::is_allowed(entity_id, state.as_ref()) {
        tracing::warn!("Alexa : {entity_id} ne se pilote pas à la voix (portail, garage)");
        return hue_json(&hue::error_response(1, &address, "unauthorized user"));
    }
    let command = match serde_json::from_slice(&body)
        .map_err(|error| error.to_string())
        .and_then(|value| hue::parse_command(&value))
    {
        Ok(command) => command,
        Err(reason) => return hue_json(&hue::error_response(2, &address, &reason)),
    };

    let (calls, promised) = hue::service_calls(entity_id, command);
    let source = client.ip().to_string();
    for (domain, service, data) in calls {
        if let Err(error) = bridge
            .core
            .call_service(domain, service, entity_id.clone(), data)
        {
            tracing::warn!("Alexa : ordre refusé : {error}");
            return hue_json(&hue::error_response(901, &address, "internal error"));
        }
        tracing::info!("Alexa : {domain}.{service} sur {entity_id} (depuis {source})");
        let detail = format!("{domain}.{service}");
        let record = AuditRecord {
            actor: "alexa",
            action: "service",
            target: Some(entity_id.as_str()),
            detail: Some(&detail),
            source: &source,
        };
        if let Err(error) = bridge.store.audit(record).await {
            tracing::error!("journal d'audit : écriture impossible ({error})");
        }
    }
    bridge.remember(number, promised);
    hue_json(&hue::success_response(number, command))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_local_network_by_default() {
        let local = Allowed::LocalNetwork;
        for ip in [
            "192.168.1.40",
            "10.0.0.2",
            "172.16.5.1",
            "127.0.0.1",
            "::1",
            "fe80::1",
            "fd00::5",
            "::ffff:192.168.1.40",
        ] {
            assert!(local.permits(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "2001:db8::1", "100.64.0.1", "::ffff:8.8.8.8"] {
            assert!(!local.permits(ip.parse().unwrap()), "{ip}");
        }
        let echo = Allowed::Only(vec!["192.168.1.40".parse().unwrap()]);
        assert!(echo.permits("192.168.1.40".parse().unwrap()));
        assert!(echo.permits("::ffff:192.168.1.40".parse().unwrap()));
        assert!(!echo.permits("192.168.1.41".parse().unwrap()));
    }

    #[test]
    fn identity_is_stable() {
        let a = Identity::new(Ipv4Addr::new(192, 168, 1, 20), 80);
        let b = Identity::new(Ipv4Addr::new(192, 168, 1, 20), 80);
        assert_eq!(a.serial, b.serial);
        assert_eq!(a.serial.len(), 12);
        assert!(a.bridge_id().starts_with("001788FFFE"));
        assert_eq!(a.mac().len(), 17);
        assert_eq!(a.location(), "http://192.168.1.20:80/description.xml");
    }
}
