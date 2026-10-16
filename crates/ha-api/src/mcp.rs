//! Brancher un assistant IA (Claude…) sur la maison, par **MCP** (Model
//! Context Protocol, étape 55) : comme le serveur MCP de Home Assistant, et
//! comme Moli OS, où des agents pilotent la maison sans pouvoir nuire.
//!
//! `POST /api/mcp` : JSON-RPC 2.0, transport « Streamable HTTP », réponses
//! en JSON (pas de flux SSE, pas de session). Trois outils :
//!
//! | Outil | Rôle |
//! |-------|------|
//! | `get_house_state` | l'état des appareils visibles du compte, par pièce |
//! | `assist` | une phrase en français, comme Assist (étape 54) |
//! | `call_service` | un ordre précis : domaine, service, appareil |
//!
//! Sécurité :
//! - un **jeton** par assistant (`ha-server user token <nom>`), avec les
//!   droits de son compte ;
//! - chaque ordre suit le chemin d'un toucher (`rest::execute`), en
//!   **assistant IA** : la garde le retient dans une pièce protégée ou aux
//!   heures calmes quel que soit le rôle du compte, et il ne peut jamais
//!   ouvrir la maison (serrure, alarme) ;
//! - 60 appels d'outils par minute et par compte ;
//! - une page web ne peut pas s'en servir : une requête qui annonce une
//!   autre origine (`Origin`) est refusée (protection contre le « DNS
//!   rebinding », demandée par la spécification MCP).

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ha_core::{Attributes, EntityId};
use serde_json::{Value, json};

use crate::ApiState;
use crate::assist::{converse, devices, explain};
use crate::auth::Authenticated;
use crate::rest::{Outcome, audit, execute};

/// Les versions du protocole connues, de la plus récente à la plus ancienne.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// Appels d'outils au plus par minute et par compte.
const MAX_CALLS_PER_MINUTE: usize = 60;
/// Réponse de `get_house_state` : au plus tant de caractères.
const MAX_STATE_CHARS: usize = 12_000;

/// Les derniers appels de chaque compte, pour les limiter.
#[derive(Debug, Default)]
pub(crate) struct Limiter(Mutex<HashMap<i64, VecDeque<Instant>>>);

impl Limiter {
    fn allow(&self, user_id: i64) -> bool {
        let now = Instant::now();
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let calls = map.entry(user_id).or_default();
        while calls
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_mins(1))
        {
            calls.pop_front();
        }
        if calls.len() >= MAX_CALLS_PER_MINUTE {
            return false;
        }
        calls.push_back(now);
        true
    }
}

fn rpc_result(id: &Value, result: Value) -> Response {
    let mut response = json!({"jsonrpc": "2.0", "id": id});
    response["result"] = result;
    Json(response).into_response()
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
        .into_response()
}

fn text_result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

/// Une page d'un autre site ne doit pas pouvoir parler au serveur MCP.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        // Un assistant IA (pas un navigateur) n'envoie pas d'origine.
        return true;
    };
    let (Ok(origin), Some(Ok(host))) = (
        origin.to_str(),
        headers.get(header::HOST).map(|host| host.to_str()),
    ) else {
        return false;
    };
    origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        == Some(host)
}

fn tools() -> Value {
    json!([
        {
            "name": "get_house_state",
            "title": "État de la maison",
            "description": "Donne l'état des appareils de la maison (lumières, volets, chauffage, \
                capteurs, personnes…), rangés par pièce, avec leur identifiant (entity_id). \
                À appeler avant de commander, pour connaître les noms et les identifiants.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "assist",
            "title": "Demander en français",
            "description": "Une phrase simple en français, comprise par la maison comme si on la \
                tapait dans Assist : « ferme le volet de la chambre », « éteins les lumières du \
                salon », « mets le chauffage à 21 degrés », « quelle est la température du salon ? ». \
                Une action par phrase. Un ordre dans une pièce protégée ou aux heures calmes attend \
                qu'un humain le valide.",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string", "maxLength": ha_assist::MAX_TEXT_CHARS}},
                "required": ["text"],
                "additionalProperties": false,
            },
        },
        {
            "name": "call_service",
            "title": "Donner un ordre précis",
            "description": "Appelle un service sur un appareil, comme Home Assistant : par exemple \
                domain « cover », service « set_cover_position », entity_id « cover.chambre », \
                data {\"position\": 40}. Les valeurs sont vérifiées par la maison. Un assistant IA \
                ne peut jamais déverrouiller une porte ni désarmer l'alarme.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "domain": {"type": "string"},
                    "service": {"type": "string"},
                    "entity_id": {"type": "string"},
                    "data": {"type": "object"},
                },
                "required": ["domain", "service", "entity_id"],
                "additionalProperties": false,
            },
        },
    ])
}

/// `POST /api/mcp`
pub(crate) async fn handle(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    auth: Authenticated,
    body: Bytes,
) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "origine refusée").into_response();
    }
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        return rpc_error(&Value::Null, -32700, "JSON illisible");
    };
    // Pas de lots (retirés de la version 2025-06-18).
    let (Some(method), true) = (
        request.get("method").and_then(Value::as_str),
        request.get("jsonrpc") == Some(&json!("2.0")),
    ) else {
        return rpc_error(&Value::Null, -32600, "requête JSON-RPC 2.0 attendue");
    };
    let Some(id) = request.get("id").cloned() else {
        // Une notification (`notifications/initialized`…) : rien à répondre.
        return StatusCode::ACCEPTED.into_response();
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .filter(|asked| VERSIONS.contains(asked))
                .unwrap_or(VERSIONS[0]);
            tracing::info!(
                "MCP : assistant IA connecté avec le compte {}",
                auth.user.username
            );
            rpc_result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "maison", "title": "Maison", "version": ha_core::VERSION},
                    "instructions": "La maison de la famille. Appelle get_house_state pour voir les \
                        appareils, puis assist ou call_service pour agir. Réponds en français.",
                }),
            )
        }
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(&id, json!({"tools": tools()})),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !matches!(name, "get_house_state" | "assist" | "call_service") {
                return rpc_error(&id, -32602, "outil inconnu");
            }
            if !state.mcp_limiter.allow(auth.user.id) {
                return rpc_result(
                    &id,
                    text_result("Trop de demandes : attends une minute.", true),
                );
            }
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            audit(&state, &auth.user.username, "mcp", None, Some(name), client).await;
            let result = match name {
                "get_house_state" => text_result(&house_state(&state, &auth), false),
                "assist" => match arguments.get("text").and_then(Value::as_str) {
                    Some(text) => {
                        let (kind, speech) = converse(&state, &auth, client, text, true).await;
                        text_result(&speech, kind == "error")
                    }
                    None => text_result("Il manque la phrase (« text »).", true),
                },
                _ => call_service(&state, &auth, client, &arguments).await,
            };
            rpc_result(&id, result)
        }
        _ => rpc_error(&id, -32601, "méthode inconnue"),
    }
}

/// `GET /api/mcp` : pas de flux d'événements (le serveur ne pousse rien).
pub(crate) async fn no_stream() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// L'état des appareils visibles, par pièce.
fn house_state(state: &ApiState, auth: &Authenticated) -> String {
    let mut devices = devices(state, auth);
    devices.sort_by(|a, b| {
        (a.area.as_deref().unwrap_or("~"), a.name.as_str())
            .cmp(&(b.area.as_deref().unwrap_or("~"), b.name.as_str()))
    });
    let mut out = String::new();
    let mut current: Option<Option<&str>> = None;
    for device in &devices {
        let area = device.area.as_deref();
        if current != Some(area) {
            let _ = writeln!(out, "\n## {}", area.unwrap_or("Sans pièce"));
            current = Some(area);
        }
        let _ = writeln!(
            out,
            "- {} ({}) : {}",
            device.name,
            device.state.entity_id(),
            ha_assist::describe(device)
        );
        if out.len() > MAX_STATE_CHARS {
            out.push_str("\n(liste coupée : trop d'appareils)\n");
            break;
        }
    }
    if out.is_empty() {
        "Aucun appareil visible pour ce compte.".into()
    } else {
        out.trim_start().to_owned()
    }
}

async fn call_service(
    state: &ApiState,
    auth: &Authenticated,
    client: SocketAddr,
    arguments: &Value,
) -> Value {
    let text = |key: &str| arguments.get(key).and_then(Value::as_str);
    let (Some(domain), Some(service), Some(entity)) =
        (text("domain"), text("service"), text("entity_id"))
    else {
        return text_result("Il faut domain, service et entity_id.", true);
    };
    let Ok(entity_id) = entity.parse::<EntityId>() else {
        return text_result("entity_id invalide (format : domaine.nom).", true);
    };
    let data: Attributes = match arguments.get("data") {
        None | Some(Value::Null) => Attributes::new(),
        Some(Value::Object(data)) => data.clone(),
        Some(_) => return text_result("data doit être un objet.", true),
    };
    if !ha_core::is_valid_name(domain) || !ha_core::is_valid_name(service) {
        return text_result("domaine ou service invalide.", true);
    }
    let summary = ha_notices::describe_order(&state.core.states, domain, service, &entity_id);
    match execute(state, auth, client, domain, service, entity_id, data, true).await {
        Ok(Outcome::Sent | Outcome::Done) => {
            text_result(&format!("C'est fait : {summary}."), false)
        }
        Ok(Outcome::Held(reason)) => text_result(
            &format!(
                "En attente de validation par un humain : {summary} ({reason}). Ne réessaie pas : \
                 la famille a été prévenue."
            ),
            false,
        ),
        Err(error) => text_result(&format!("Pas fait : {}.", explain(error)), true),
    }
}
