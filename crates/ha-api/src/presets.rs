//! Les automatisations toutes prêtes (étape 85) : départ, retour, nuit,
//! matin, vacances, à cocher dans la liste des automatisations.
//!
//! - `GET /api/automations/presets` : chacune, avec ce qui est coché, et ce
//!   que la maison a (lumières, volets, chauffage, ouvertures) ;
//! - `PUT /api/automations/presets/{nom}` avec `{"enabled": true,
//!   "options": {…}}` : fabrique l'automatisation `pret_{nom}` d'après les
//!   appareils de la maison, et l'enregistre comme une autre (mêmes droits,
//!   même confirmation du mot de passe, mêmes vérifications, une version de
//!   plus). Décochée, elle est gardée, désactivée.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use ha_auth::Permission;
use ha_automation::presets::{Device, House, Opening, Options, Preset};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ApiState;
use crate::auth::Authenticated;
use crate::automations::{automation_error, editor, notify, require_confirmation};
use crate::error::ApiError;
use crate::rest::{audit, parse_json};
use crate::roles;

/// Les mots d'une pièce qui en font une chambre, ou une entrée.
const BEDROOM: &[&str] = &["chambre", "dortoir"];
const ENTRANCE: &[&str] = &["entree", "couloir", "hall", "vestibule"];

fn has_word(text: &str, words: &[&str]) -> bool {
    ha_assist::normalize(text)
        .split(' ')
        .any(|word| words.iter().any(|w| word.starts_with(w)))
}

/// Ce que la maison a, d'après ses états et ses pièces.
pub(crate) fn house(state: &ApiState) -> House {
    let mut areas: HashMap<String, (&str, Option<&str>)> = HashMap::new();
    for area in &state.areas {
        for member in &area.entities {
            areas.insert(
                member.entity_id.to_string(),
                (area.name.as_str(), member.name.as_deref()),
            );
        }
    }
    let mut house = House::default();
    let mut notify = Vec::new();
    let mut entities = state.core.states.all();
    entities.sort_by(|a, b| a.entity_id().as_str().cmp(b.entity_id().as_str()));
    for entity in &entities {
        let id = entity.entity_id().to_string();
        let attributes = entity.attributes();
        let class = attributes.get("device_class").and_then(Value::as_str);
        let placed = areas.get(&id);
        let name = attributes
            .get("friendly_name")
            .and_then(Value::as_str)
            .or_else(|| placed.and_then(|(_, name)| *name))
            .unwrap_or(entity.entity_id().object_id())
            .to_owned();
        let room = placed.map_or("", |(area, _)| area);
        let device = || Device {
            entity_id: id.clone(),
            name: name.clone(),
            bedroom: has_word(room, BEDROOM) || has_word(&name, BEDROOM),
            entrance: has_word(room, ENTRANCE) || has_word(&name, ENTRANCE),
        };
        let opening = |open_state: &str| Opening {
            entity_id: id.clone(),
            name: name.clone(),
            open_state: open_state.to_owned(),
        };
        match entity.entity_id().domain() {
            // Une prise Hue (`light` de classe `outlet`) n'est pas une lumière :
            // le frigo ne s'éteint pas au départ.
            "light" if class != Some("outlet") => house.lights.push(device()),
            "cover" if matches!(class, Some("garage" | "gate" | "door")) => {
                house.openings.push(opening("open"));
            }
            "cover" => house.covers.push(device()),
            "climate" => house.climates.push(device()),
            "binary_sensor"
                if matches!(class, Some("door" | "window" | "opening" | "garage_door")) =>
            {
                house.openings.push(opening("on"));
            }
            "notify" => notify.push(id.clone()),
            "switch" if id == "switch.simulation_presence" => house.simulation = Some(id.clone()),
            "zone" if id == "zone.home" => house.people = true,
            "sun" if id == "sun.sun" => house.sun = true,
            _ => {}
        }
    }
    // Les notifications de l'appli d'abord (étape 50), sinon ntfy.
    house.notify = notify
        .iter()
        .find(|id| *id == "notify.appli")
        .or_else(|| notify.first())
        .cloned();
    house
}

/// Les réglages gardés pour chaque automatisation toute prête.
async fn stored_options(state: &ApiState) -> Result<HashMap<String, Options>, ApiError> {
    let rows = state
        .store
        .automation_presets()
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(rows
        .into_iter()
        .filter_map(|(name, options)| {
            serde_json::from_str::<Options>(&options)
                .ok()
                .map(|options| (name, options))
        })
        .collect())
}

/// `GET /api/automations/presets`
pub(crate) async fn list(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::ReadAutomations)?;
    let automations = editor(&state)?;
    let entries = automations
        .editor
        .entries()
        .await
        .map_err(automation_error)?;
    let mut stored = stored_options(&state).await?;
    let house = house(&state);
    let presets: Vec<Value> = Preset::ALL
        .into_iter()
        .map(|preset| {
            let id = preset.automation_id();
            let entry = entries.iter().find(|entry| entry.id == id);
            let options = stored
                .remove(preset.name())
                .unwrap_or_default()
                .or(preset.defaults(&house));
            json!({
                "name": preset.name(),
                "id": id,
                "title": preset.title(),
                "when": preset.when(),
                "unavailable": preset.unavailable(&house),
                "exists": entry.is_some(),
                "enabled": entry
                    .and_then(|entry| entry.config.as_ref())
                    .is_some_and(ha_automation::AutomationConfig::enabled),
                "options": options,
            })
        })
        .collect();
    let named = |devices: &[Device]| -> Vec<Value> {
        devices
            .iter()
            .map(|d| json!({ "entity_id": d.entity_id, "name": d.name }))
            .collect()
    };
    Ok(Json(json!({
        "editable": automations.editable && auth.user.can(Permission::EditAutomations),
        "presets": presets,
        "house": {
            "lights": named(&house.lights),
            "covers": house.covers.len(),
            "bedroom_covers": house.covers.iter().filter(|c| c.bedroom).count(),
            "bedroom_lights": house.lights.iter().filter(|l| l.bedroom).count(),
            "climates": house.climates.len(),
            "openings": house.openings.len(),
            "notify": house.notify.is_some(),
            "simulation": house.simulation.is_some(),
            "people": house.people,
            "sun": house.sun,
        },
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresetRequest {
    enabled: bool,
    #[serde(default)]
    options: Options,
}

/// `PUT /api/automations/presets/{nom}` (confirmation requise).
pub(crate) async fn save(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::EditAutomations)?;
    let preset = Preset::from_name(&name).ok_or(ApiError::NotFound)?;
    let editor = require_confirmation(&state, &auth)?;
    let PresetRequest { enabled, options } = parse_json(&body)?;
    let house = house(&state);
    let options = options.or(preset.defaults(&house));
    let config = ha_automation::presets::build(preset, &options, &house, enabled)
        .map_err(ApiError::BadRequest)?;
    let username = &auth.user.username;
    editor
        .save(&config, username)
        .await
        .map_err(automation_error)?;
    let stored =
        serde_json::to_string(&options).map_err(|error| ApiError::Internal(error.to_string()))?;
    state
        .store
        .save_automation_preset(preset.name(), &stored, username)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let verb = if enabled { "activée" } else { "désactivée" };
    tracing::info!("automatisation toute prête « {name} » {verb} par {username}");
    audit(
        &state,
        username,
        "automation_preset",
        Some(config.id()),
        Some(config.alias()),
        client,
    )
    .await;
    notify(
        &state,
        &format!("« {} » {verb} par {username}", config.alias()),
    );
    Ok(Json(json!({
        "id": config.id(),
        "enabled": enabled,
        "options": options,
        "config": config.to_json(),
    })))
}
