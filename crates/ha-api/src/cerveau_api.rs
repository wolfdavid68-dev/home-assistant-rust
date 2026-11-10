//! Ce que le tableau de bord demande au cerveau (docs/cerveau.md) :
//!
//! - `GET /api/cerveau` : est-il branché, combien de questions aujourd'hui ;
//! - `GET` / `PUT /api/cerveau/preferences` : le texte que la famille écrit
//!   pour lui (« cosy = lumière chaude à 30 % ») ;
//! - `POST /api/cerveau/explique` : « Pourquoi ? » sur une ligne du journal
//!   de bord.
//!
//! Et le résumé du matin ([`morning_summary`]), envoyé chaque jour à l'heure
//! choisie.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use ha_auth::{Permission, Role};
use ha_core::{Attributes, EntityId};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ApiState;
use crate::auth::Authenticated;
use crate::cerveau::{Failure, MAX_CONTEXTE, MAX_PREFERENCES, Question, cut};
use crate::error::ApiError;
use crate::json::format_timestamp;
use crate::rest::parse_json;
use crate::roles;

/// Les lignes du journal lues autour de celle qu'on explique.
const AROUND: Duration = Duration::from_mins(30);
/// Au plus autant de lignes autour.
const AROUND_LIMIT: u32 = 60;
/// La question ajoutée à « Pourquoi ? », au plus.
const MAX_QUESTION: usize = 200;
/// La notification du matin sur la tablette.
const MORNING_NOTICE: &str = "persistent_notification.resume_du_matin";
/// Les téléphones (notifications de l'application).
const PHONES: &str = "notify.appli";
const MORNING_TITLE: &str = "Résumé du matin";
const MORNING_ASK: &str = "Fais le résumé du matin pour la famille, en trois ou quatre phrases \
     courtes : la météo du jour, ce qui est prévu aujourd'hui dans l'agenda, les colis \
     attendus et l'énergie consommée hier. Passe ce qui manque.";

/// `GET /api/cerveau` : le cerveau est-il branché ? Pour un compte de la
/// famille, combien de questions lui ont été posées.
pub(crate) async fn overview(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Json<Value> {
    let family = auth.user.role != Role::Guest;
    let usage = state
        .cerveau
        .get()
        .filter(|_| family)
        .map(|cerveau| cerveau.usage(Instant::now()));
    Json(json!({
        "installe": state.cerveau.get().is_some(),
        "usage": usage,
    }))
}

/// `GET /api/cerveau/preferences` : le texte, qui l'a écrit, et quand.
pub(crate) async fn preferences(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    family_only(&auth)?;
    let saved = state.store.cerveau_preferences().await?;
    Ok(Json(match saved {
        Some(saved) => json!({
            "texte": saved.text,
            "modifie_le": format_timestamp(saved.updated_at),
            "modifie_par": saved.updated_by,
            "max": MAX_PREFERENCES,
        }),
        None => json!({ "texte": "", "max": MAX_PREFERENCES }),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPreferences {
    texte: String,
}

/// `PUT /api/cerveau/preferences` avec `{"texte": "…"}`.
pub(crate) async fn save_preferences(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    family_only(&auth)?;
    let NewPreferences { texte } = parse_json(&body)?;
    let text = tidy_preferences(&texte)?;
    state
        .store
        .save_cerveau_preferences(&text, &auth.user.username)
        .await?;
    tracing::info!("préférences du cerveau changées par {}", auth.user.username);
    Ok(Json(json!({ "texte": text })))
}

/// Les préférences, sans les espaces autour ni les fins de ligne Windows.
fn tidy_preferences(text: &str) -> Result<String, ApiError> {
    let text = text.replace("\r\n", "\n");
    let text = text.trim();
    if text.chars().count() > MAX_PREFERENCES {
        return Err(ApiError::BadRequest(format!(
            "{MAX_PREFERENCES} caractères au plus"
        )));
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ApiError::BadRequest("caractère de contrôle refusé".into()));
    }
    Ok(text.to_owned())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Explain {
    /// Le numéro de la ligne du journal (`id` dans `GET /api/logbook`).
    id: i64,
    /// Une précision (« pourquoi si tôt ? »), sinon « Pourquoi ? ».
    #[serde(default)]
    question: Option<String>,
}

/// `POST /api/cerveau/explique` avec `{"id": 42}` : le cerveau explique une
/// ligne du journal de bord, avec ce qui s'est passé autour.
pub(crate) async fn explain(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadHistory)?;
    family_only(&auth)?;
    let Explain { id, question } = parse_json(&body)?;
    let question = question.as_deref().map(str::trim).unwrap_or_default();
    if question.chars().count() > MAX_QUESTION {
        return Err(ApiError::BadRequest(format!(
            "question : {MAX_QUESTION} caractères au plus"
        )));
    }
    let cerveau = state
        .cerveau
        .get()
        .ok_or_else(|| ApiError::Unavailable("le cerveau n'est pas branché".into()))?;
    let entry = state
        .store
        .logbook_entry(id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let since = entry
        .at
        .checked_sub(AROUND)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let until = entry.at.checked_add(AROUND).unwrap_or(entry.at);
    let mut around = state
        .store
        .logbook(since, until, None, AROUND_LIMIT)
        .await?;
    around.reverse();
    let lines: Vec<String> = around
        .iter()
        .map(|line| {
            let mark = if line.id == entry.id { "→ " } else { "" };
            logbook_line(mark, line)
        })
        .collect();
    let contexte = format!(
        "Le journal de bord de la maison autour de la ligne à expliquer (marquée →) :\n{}",
        lines.join("\n")
    );
    let texte = if question.is_empty() {
        format!(
            "Pourquoi « {} : {} » à {} ? Explique en deux phrases.",
            entry.name,
            entry.message,
            local_time(entry.at, "%H:%M")
        )
    } else {
        format!(
            "À propos de « {} : {} » à {} : {question}",
            entry.name,
            entry.message,
            local_time(entry.at, "%H:%M")
        )
    };
    let preferences = saved_preferences(&state).await;
    let asked = Question {
        texte: &texte,
        avant: Vec::new(),
        preferences: &preferences,
        contexte: &contexte,
    };
    match cerveau.ask(&auth.user.username, &asked).await {
        Ok(reponse) => Ok(Json(json!({ "reponse": reponse }))),
        Err(Failure::Limit) => Err(ApiError::TooManyRequests {
            retry_after: Duration::from_mins(10),
        }),
        Err(Failure::NoReply) => Err(ApiError::DeviceRefused("le cerveau n'a pas répondu".into())),
    }
}

/// Une ligne du journal pour le cerveau : « 07:02:13 Volet salon — s'est
/// ouvert (par l'automatisation Lever) ».
fn logbook_line(mark: &str, line: &ha_storage::LogbookEntry) -> String {
    let cause = line
        .cause
        .as_deref()
        .map(|cause| format!(" ({cause})"))
        .unwrap_or_default();
    format!(
        "{mark}{} {} — {}{cause}",
        local_time(line.at, "%H:%M:%S"),
        line.name,
        line.message
    )
}

/// Les invités ne parlent pas au cerveau.
fn family_only(auth: &Authenticated) -> Result<(), ApiError> {
    if auth.user.role == Role::Guest {
        return Err(ApiError::Forbidden("réservé à la famille".into()));
    }
    Ok(())
}

/// Les préférences de la maison, ou rien si elles sont illisibles.
pub(crate) async fn saved_preferences(state: &ApiState) -> String {
    match state.store.cerveau_preferences().await {
        Ok(saved) => saved.map(|saved| saved.text).unwrap_or_default(),
        Err(error) => {
            tracing::warn!("cerveau : préférences illisibles ({error})");
            String::new()
        }
    }
}

/// L'heure locale de `time`, au format `pattern`.
fn local_time(time: SystemTime, pattern: &str) -> String {
    jiff::Timestamp::try_from(time).map_or_else(
        |_| "?".to_owned(),
        |t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime(pattern)
                .to_string()
        },
    )
}

/// Le résumé du matin, chaque jour à `at` (heure locale) : demandé au
/// cerveau, puis envoyé aux téléphones et posé sur la tablette. Tourne
/// jusqu'à ce que la tâche soit arrêtée.
pub async fn morning_summary(state: Arc<ApiState>, at: jiff::civil::Time) {
    tracing::info!("résumé du matin : chaque jour à {}", at.strftime("%H:%M"));
    loop {
        tokio::time::sleep(until(at)).await;
        let Some(cerveau) = state.cerveau.get() else {
            continue;
        };
        let contexte = morning_context(&state).await;
        let preferences = saved_preferences(&state).await;
        let asked = Question {
            texte: MORNING_ASK,
            avant: Vec::new(),
            preferences: &preferences,
            contexte: &contexte,
        };
        match cerveau.ask("matin", &asked).await {
            Ok(summary) => send_morning(&state, &summary),
            Err(failure) => tracing::warn!("résumé du matin : pas de réponse ({failure:?})"),
        }
    }
}

/// Temps jusqu'à la prochaine heure `at` (heure locale, changement d'heure
/// compris).
fn until(at: jiff::civil::Time) -> Duration {
    let now = jiff::Zoned::now();
    let today = now.date().to_datetime(at);
    let next = if today > now.datetime() {
        today
    } else {
        today
            .checked_add(jiff::Span::new().days(1))
            .unwrap_or(today)
    };
    next.to_zoned(now.time_zone().clone())
        .ok()
        .and_then(|next| now.duration_until(&next).try_into().ok())
        .unwrap_or(Duration::from_hours(24))
}

/// Ce que le cerveau lit pour le résumé : la météo, l'agenda du jour, les
/// colis et l'énergie d'hier.
async fn morning_context(state: &ApiState) -> String {
    let now = jiff::Zoned::now();
    let energy =
        match ha_energy::report(&state.store, &state.core.states, &state.energy, 2, &now).await {
            Ok(report) => report
                .days
                .iter()
                .rev()
                .nth(1)
                .map(|day| {
                    let sources: Vec<&str> = report
                        .sources
                        .iter()
                        .map(|source| source.name.as_str())
                        .collect();
                    json!({ "date": day.date, "sources": sources, "kwh": day.kwh })
                })
                .filter(|_| report.configured),
            Err(error) => {
                tracing::warn!("résumé du matin : énergie illisible ({error})");
                None
            }
        };
    let today = now.date().to_string();
    let states = state.core.states.all();
    let states: Vec<(&EntityId, &str, &Attributes)> = states
        .iter()
        .map(|s| (s.entity_id(), s.state(), s.attributes()))
        .collect();
    morning_lines(&states, &today, energy.as_ref())
}

/// Les lignes du contexte du matin, à partir des états de la maison.
fn morning_lines(
    states: &[(&EntityId, &str, &Attributes)],
    today: &str,
    energy: Option<&Value>,
) -> String {
    let mut lines = Vec::new();
    for (entity_id, state, attributes) in states {
        let name = attributes
            .get("friendly_name")
            .and_then(Value::as_str)
            .unwrap_or(entity_id.as_str());
        match entity_id.domain() {
            "weather" => {
                let forecast: Vec<&Value> = attributes
                    .get("forecast")
                    .and_then(Value::as_array)
                    .map(|days| days.iter().take(2).collect())
                    .unwrap_or_default();
                lines.push(format!(
                    "Météo {name} : {state}, {} degrés, prévisions {}",
                    attributes.get("temperature").unwrap_or(&Value::Null),
                    json!(forecast)
                ));
            }
            "calendar" => {
                for event in today_events(attributes, today) {
                    lines.push(format!("Agenda {name} : {event}"));
                }
            }
            "sensor" if entity_id.as_str().starts_with("sensor.colis") => {
                let mut details = (*attributes).clone();
                details.remove("friendly_name");
                details.remove("icon");
                lines.push(format!("Colis {name} : {state} {}", json!(details)));
            }
            _ => {}
        }
    }
    if let Some(energy) = energy {
        lines.push(format!("Énergie d'hier : {energy}"));
    }
    cut(&lines.join("\n"), MAX_CONTEXTE)
}

/// Les événements d'un agenda qui touchent `today` (`2026-11-10`).
fn today_events(attributes: &Attributes, today: &str) -> Vec<Value> {
    let day = |event: &Value, field: &str| {
        event
            .get(field)
            .and_then(Value::as_str)
            .and_then(|text| text.get(..10))
            .map(str::to_owned)
    };
    attributes
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter(|event| {
                    let start = day(event, "start");
                    let end = day(event, "end").or_else(|| start.clone());
                    start.is_some_and(|start| start.as_str() <= today)
                        && end.is_some_and(|end| end.as_str() >= today)
                })
                .map(|event| {
                    json!({
                        "titre": event.get("summary"),
                        "debut": event.get("start"),
                        "fin": event.get("end"),
                        "journee": event.get("all_day"),
                        "lieu": event.get("location"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Envoie le résumé aux téléphones et le pose sur la tablette.
fn send_morning(state: &ApiState, summary: &str) {
    if let Ok(notice) = MORNING_NOTICE.parse::<EntityId>()
        && let Err(error) =
            ha_notices::notify(&state.core.states, &notice, Some(MORNING_TITLE), summary)
    {
        tracing::warn!("résumé du matin : tablette ({error})");
    }
    let Ok(phones) = PHONES.parse::<EntityId>() else {
        return;
    };
    if !state.core.states.contains(&phones) {
        return;
    }
    let mut data = Attributes::new();
    data.insert("title".into(), MORNING_TITLE.into());
    data.insert("message".into(), summary.into());
    if let Err(error) = state
        .core
        .call_service("notify", "send_message", phones, data)
    {
        tracing::warn!("résumé du matin : téléphones ({error})");
    }
    tracing::info!("résumé du matin envoyé");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attributes(value: Value) -> Attributes {
        match value {
            Value::Object(map) => map,
            _ => Attributes::new(),
        }
    }

    #[test]
    fn preferences_are_tidied() {
        assert_eq!(tidy_preferences("  a\r\nb  ").unwrap(), "a\nb");
        assert!(tidy_preferences(&"x".repeat(MAX_PREFERENCES + 1)).is_err());
        assert!(tidy_preferences("a\u{7}b").is_err());
        assert_eq!(
            tidy_preferences("é".repeat(MAX_PREFERENCES).as_str())
                .unwrap()
                .chars()
                .count(),
            MAX_PREFERENCES
        );
    }

    #[test]
    fn only_today_s_events() {
        let calendar = attributes(json!({
            "events": [
                { "summary": "Hier", "start": "2026-11-09T10:00:00", "end": "2026-11-09T11:00:00" },
                { "summary": "Dentiste", "start": "2026-11-10T09:00:00", "end": "2026-11-10T09:30:00" },
                { "summary": "Vacances", "start": "2026-11-08", "end": "2026-11-12", "all_day": true },
                { "summary": "Demain", "start": "2026-11-11T08:00:00" },
            ]
        }));
        let titles: Vec<Value> = today_events(&calendar, "2026-11-10")
            .into_iter()
            .map(|event| event["titre"].clone())
            .collect();
        assert_eq!(titles, vec![json!("Dentiste"), json!("Vacances")]);
    }

    #[test]
    fn the_morning_context() {
        let weather: EntityId = "weather.maison".parse().unwrap();
        let parcels: EntityId = "sensor.colis_amazon".parse().unwrap();
        let light: EntityId = "light.salon".parse().unwrap();
        let weather_attributes = attributes(json!({
            "friendly_name": "Maison",
            "temperature": 8,
            "forecast": [{"condition": "rainy"}, {"condition": "sunny"}, {"condition": "cloudy"}],
        }));
        let parcel_attributes = attributes(json!({ "friendly_name": "Colis", "colis": ["Plaid"] }));
        let empty = Attributes::new();
        let states = [
            (&weather, "rainy", &weather_attributes),
            (&parcels, "1", &parcel_attributes),
            (&light, "on", &empty),
        ];
        let energy = json!({ "date": "2026-11-09", "kwh": [12.5] });
        let text = morning_lines(&states, "2026-11-10", Some(&energy));
        assert_eq!(
            text,
            "Météo Maison : rainy, 8 degrés, prévisions [{\"condition\":\"rainy\"},{\"condition\":\"sunny\"}]\n\
             Colis Colis : 1 {\"colis\":[\"Plaid\"]}\n\
             Énergie d'hier : {\"date\":\"2026-11-09\",\"kwh\":[12.5]}"
        );
    }
}
