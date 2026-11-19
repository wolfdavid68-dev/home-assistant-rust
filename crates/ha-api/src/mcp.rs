//! Brancher un assistant IA (Claude…) sur la maison, par **MCP** (Model
//! Context Protocol, étape 55) : comme le serveur MCP de Home Assistant, et
//! comme Moli OS, où des agents pilotent la maison sans pouvoir nuire.
//!
//! `POST /api/mcp` : JSON-RPC 2.0, transport « Streamable HTTP », réponses
//! en JSON (pas de flux SSE, pas de session). Quatre outils :
//!
//! | Outil | Rôle |
//! |-------|------|
//! | `get_house_state` | l'état des appareils visibles du compte, par pièce |
//! | `get_agenda` | les événements des agendas, du jour ou des jours à venir |
//! | `assist` | une phrase en français, comme Assist (étape 54) |
//! | `call_service` | un ordre précis : domaine, service, appareil |
//!
//! Sécurité :
//! - un **jeton** par assistant (`ha-server user token <nom>`), avec les
//!   droits de son compte ; il ne marche que sur cette route (le tableau de
//!   bord, l'API et le WebSocket le refusent) ;
//! - chaque ordre suit le chemin d'un toucher (`rest::execute`), en
//!   **assistant IA** : la garde le retient dans une pièce protégée ou aux
//!   heures calmes quel que soit le rôle du compte, et il ne touche jamais
//!   à ce qui ferme la maison (serrure, alarme, sirène, garage, portail,
//!   porte) ;
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
use crate::auth::{AgentAuthenticated, Authenticated};
use crate::rest::{Outcome, audit, execute};
use crate::roles;

/// Les versions du protocole connues, de la plus récente à la plus ancienne.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// Appels d'outils au plus par minute et par compte.
const MAX_CALLS_PER_MINUTE: usize = 60;
/// Réponse de `get_house_state` ou `get_agenda` : au plus tant de caractères.
const MAX_STATE_CHARS: usize = 12_000;
/// `get_agenda` : au plus tant de jours.
const MAX_AGENDA_DAYS: i64 = 14;
/// La description d'un événement : au plus tant de caractères.
const MAX_DESCRIPTION_CHARS: usize = 200;

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
            "name": "get_agenda",
            "title": "Agenda de la maison",
            "description": "Donne les événements des agendas de la maison (agenda de la famille, \
                agendas partagés), en heure locale, triés par heure : aujourd'hui (days = 1, par \
                défaut) ou les jours à venir (days jusqu'à 14). Un compte invité ne voit pas l'agenda.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": {"type": "integer", "minimum": 1, "maximum": MAX_AGENDA_DAYS, "default": 1},
                },
                "additionalProperties": false,
            },
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
                ne commande jamais une serrure, l'alarme, une sirène, une porte de garage, un portail ou une porte.",
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
    AgentAuthenticated(auth): AgentAuthenticated,
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
                        appareils, get_agenda pour l'agenda, puis assist ou call_service pour agir. \
                        Réponds en français.",
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
            if !matches!(
                name,
                "get_house_state" | "get_agenda" | "assist" | "call_service"
            ) {
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
                "get_agenda" => match agenda_days(&arguments) {
                    Some(days) => text_result(&agenda(&state, &auth, days), false),
                    None => text_result(
                        &format!("days doit être un nombre entier de 1 à {MAX_AGENDA_DAYS}."),
                        true,
                    ),
                },
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
        // Les autres noms (étape 94), pour que l'assistant les reconnaisse.
        let aliases = if device.aliases.is_empty() {
            String::new()
        } else {
            format!(" [autres noms : {}]", device.aliases.join(", "))
        };
        let _ = writeln!(
            out,
            "- {} ({}){aliases} : {}",
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

/// Le nombre de jours demandé à `get_agenda` (1 par défaut).
fn agenda_days(arguments: &Value) -> Option<i64> {
    match arguments.get("days") {
        None | Some(Value::Null) => Some(1),
        Some(days) => days
            .as_i64()
            .filter(|days| (1..=MAX_AGENDA_DAYS).contains(days)),
    }
}

/// Les événements des agendas visibles du compte, d'aujourd'hui à
/// `days` jours plus tard (heure locale de la maison).
fn agenda(state: &ApiState, auth: &Authenticated, days: i64) -> String {
    let today = jiff::Zoned::now().date();
    let last = today
        .checked_add(jiff::Span::new().days(days - 1))
        .unwrap_or(today);
    let visible: Vec<_> = state
        .core
        .states
        .all()
        .into_iter()
        .filter(|entity| entity.entity_id().domain() == "calendar")
        .filter(|entity| roles::can_see(&auth.user, entity.entity_id(), Some(entity)))
        .collect();
    if visible.is_empty() {
        return "Aucun agenda visible pour ce compte (un compte invité ne voit pas l'agenda)."
            .into();
    }
    let calendars: Vec<(String, &Attributes)> = visible
        .iter()
        .map(|entity| {
            let name = entity
                .attributes()
                .get("friendly_name")
                .and_then(Value::as_str)
                .map_or_else(|| entity.entity_id().to_string(), str::to_owned);
            (name, entity.attributes())
        })
        .collect();
    agenda_text(&calendars, today, last)
}

/// Les événements qui touchent les jours `first` à `last` (inclus), en
/// lignes de texte triées par heure.
///
/// Les dates des agendas sont en heure locale, en texte
/// (`2026-10-15T18:30:00`) ; la fin d'un événement d'une journée est
/// exclue (le lendemain à minuit).
fn agenda_text(
    calendars: &[(String, &Attributes)],
    first: jiff::civil::Date,
    last: jiff::civil::Date,
) -> String {
    let from = format!("{first}T00:00:00");
    let until = last.tomorrow().map_or_else(
        |_| format!("{last}T23:59:59"),
        |day| format!("{day}T00:00:00"),
    );
    let text = |event: &Value, field: &str| {
        event
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let mut events: Vec<(String, String)> = Vec::new();
    for (calendar, attributes) in calendars {
        let Some(entries) = attributes.get("events").and_then(Value::as_array) else {
            continue;
        };
        for event in entries {
            let Some(start) = text(event, "start").filter(|start| start.len() >= 10) else {
                continue;
            };
            let end = text(event, "end").unwrap_or_else(|| start.clone());
            // Un événement sans durée compte s'il commence dans la période.
            let touches = start < until && (end > from || (end <= start && start >= from));
            if !touches {
                continue;
            }
            let all_day = event.get("all_day").and_then(Value::as_bool) == Some(true);
            let line = event_line(event, calendar, &start, &end, all_day);
            events.push((start, line));
        }
    }
    let period = if first == last {
        format!("Agenda du {first} (heure locale)")
    } else {
        format!("Agenda du {first} au {last} (heure locale)")
    };
    if events.is_empty() {
        return format!("{period} : rien de prévu.");
    }
    events.sort();
    let mut out = format!("{period} :\n");
    for (_, line) in &events {
        out.push_str(line);
        out.push('\n');
        if out.len() > MAX_STATE_CHARS {
            out.push_str("(liste coupée : trop d'événements)\n");
            break;
        }
    }
    out
}

/// Une ligne : `- 2026-11-20 18:30 → 19:00 · Dentiste (Agenda) · lieu : …`.
fn event_line(event: &Value, calendar: &str, start: &str, end: &str, all_day: bool) -> String {
    let day = |text: &str| text.get(..10).unwrap_or(text).to_owned();
    let time = |text: &str| text.get(11..16).unwrap_or_default().to_owned();
    let when = if all_day {
        // La fin est exclue : le dernier jour est la veille.
        let last = day(end)
            .parse::<jiff::civil::Date>()
            .ok()
            .and_then(|end| end.yesterday().ok())
            .map(|last| last.to_string())
            .filter(|last| last.as_str() > day(start).as_str());
        match last {
            Some(last) => format!("{} → {last} (journées entières)", day(start)),
            None => format!("{} (journée entière)", day(start)),
        }
    } else if end <= start {
        format!("{} {}", day(start), time(start))
    } else if day(end) == day(start) {
        format!("{} {} → {}", day(start), time(start), time(end))
    } else {
        format!(
            "{} {} → {} {}",
            day(start),
            time(start),
            day(end),
            time(end)
        )
    };
    let field = |name: &str| {
        event
            .get(name)
            .and_then(Value::as_str)
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| !text.is_empty())
    };
    let title = field("summary").unwrap_or_else(|| "(sans titre)".into());
    let mut line = format!("- {when} · {title} ({calendar})");
    if let Some(location) = field("location") {
        let _ = write!(line, " · lieu : {location}");
    }
    if let Some(description) = field("description") {
        let mut short: String = description.chars().take(MAX_DESCRIPTION_CHARS).collect();
        if short.len() < description.len() {
            short.push('…');
        }
        let _ = write!(line, " · note : {short}");
    }
    line
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

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(events: Value) -> Attributes {
        let mut attributes = Attributes::new();
        attributes.insert("events".into(), events);
        attributes
    }

    fn date(text: &str) -> jiff::civil::Date {
        text.parse().unwrap()
    }

    #[test]
    fn the_agenda_keeps_what_touches_the_period() {
        let family = calendar(json!([
            {"summary": "Hier", "start": "2026-11-19T10:00:00", "end": "2026-11-19T11:00:00"},
            {"summary": "Dentiste", "start": "2026-11-20T18:30:00", "end": "2026-11-20T19:00:00",
             "location": "Colmar", "description": "Penser\n à la carte"},
            {"summary": "Vacances", "start": "2026-11-18T00:00:00", "end": "2026-11-22T00:00:00",
             "all_day": true},
            {"summary": "Fini hier", "start": "2026-11-19T00:00:00", "end": "2026-11-20T00:00:00",
             "all_day": true},
            {"summary": "Rappel", "start": "2026-11-20T08:00:00"},
            {"summary": "Demain", "start": "2026-11-21T09:00:00", "end": "2026-11-21T10:00:00"},
        ]));
        let shared = calendar(json!([
            {"summary": "Match", "start": "2026-11-20T07:00:00", "end": "2026-11-20T09:00:00"},
            {"summary": "Anniversaire", "start": "2026-11-20T00:00:00",
             "end": "2026-11-21T00:00:00", "all_day": true},
        ]));
        let calendars = [
            ("Agenda de la maison".to_owned(), &family),
            ("Club".to_owned(), &shared),
        ];
        let today = agenda_text(&calendars, date("2026-11-20"), date("2026-11-20"));
        assert_eq!(
            today,
            "Agenda du 2026-11-20 (heure locale) :\n\
             - 2026-11-18 → 2026-11-21 (journées entières) · Vacances (Agenda de la maison)\n\
             - 2026-11-20 (journée entière) · Anniversaire (Club)\n\
             - 2026-11-20 07:00 → 09:00 · Match (Club)\n\
             - 2026-11-20 08:00 · Rappel (Agenda de la maison)\n\
             - 2026-11-20 18:30 → 19:00 · Dentiste (Agenda de la maison) · lieu : Colmar · \
             note : Penser à la carte\n"
        );
        let two_days = agenda_text(&calendars, date("2026-11-20"), date("2026-11-21"));
        assert!(
            two_days.starts_with("Agenda du 2026-11-20 au 2026-11-21"),
            "{two_days}"
        );
        assert!(two_days.contains("· Demain"), "{two_days}");
        assert!(!two_days.contains("Hier"), "{two_days}");
        assert!(!two_days.contains("Fini hier"), "{two_days}");
        let empty = agenda_text(&calendars, date("2026-12-24"), date("2026-12-24"));
        assert_eq!(
            empty,
            "Agenda du 2026-12-24 (heure locale) : rien de prévu."
        );
    }

    #[test]
    fn a_long_note_is_cut() {
        let long = "a".repeat(500);
        let attributes = calendar(json!([
            {"summary": "Réunion", "start": "2026-11-20T10:00:00", "end": "2026-11-20T11:00:00",
             "description": long},
        ]));
        let text = agenda_text(
            &[("Agenda".to_owned(), &attributes)],
            date("2026-11-20"),
            date("2026-11-20"),
        );
        assert!(
            text.contains(&format!("note : {}…", "a".repeat(200))),
            "{text}"
        );
    }

    #[test]
    fn the_number_of_days_is_checked() {
        assert_eq!(agenda_days(&json!({})), Some(1));
        assert_eq!(agenda_days(&json!({"days": 7})), Some(7));
        assert_eq!(agenda_days(&json!({"days": 0})), None);
        assert_eq!(agenda_days(&json!({"days": 15})), None);
        assert_eq!(agenda_days(&json!({"days": "2"})), None);
    }
}
