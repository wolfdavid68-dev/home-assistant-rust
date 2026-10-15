//! Le dessin de la maison, au centre du style « plan d'architecte ».
//!
//! Comme dans les tableaux de bord isométriques de Home Assistant : un fond
//! (de jour, et de nuit si on veut), des **calques** transparents de la même
//! taille qui n'apparaissent que si un appareil est dans un certain état (le
//! garage ouvert, le porche allumé), et des **valeurs** posées sur le dessin.
//!
//! | Méthode | Chemin | Rôle |
//! |---------|--------|------|
//! | GET | `/api/home/drawing` | L'assemblage et la liste des images |
//! | PUT | `/api/home/drawing` | Change l'assemblage (administrateur) |
//! | GET | `/api/home/images/{name}` | Une image |
//! | PUT / DELETE | `/api/home/images/{name}` | Ajoute, remplace, retire une image (administrateur) |
//!
//! Sécurité : le **contenu** décide du type d'une image, jamais ce que le
//! navigateur annonce. Seuls PNG, JPEG et WebP passent, reconnus à leurs
//! premiers octets ; un SVG (qui peut contenir du script) ou un HTML déguisé
//! en image est refusé. Les images repartent avec `nosniff` et la politique
//! de sécurité de toutes les réponses.

use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ha_auth::{Permission, User};
use ha_core::EntityId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ApiState;
use crate::auth::Authenticated;
use crate::error::ApiError;
use crate::json::format_timestamp;
use crate::rest::{audit, parse_json};
use crate::roles;

/// Taille maximale d'une image : de quoi garder un beau dessin, pas une
/// photo de 20 mégapixels.
pub(crate) const MAX_IMAGE_SIZE: usize = 3 * 1024 * 1024;
const MAX_IMAGES: usize = 16;
const MAX_NAME_CHARS: usize = 40;
const MAX_LAYERS: usize = 16;
const MAX_LABELS: usize = 30;
const MAX_ACTIONS: usize = 12;
const MAX_CHARTS: usize = 6;
/// Les couleurs d'une courbe (celles du thème : Material dans « Maquette »).
const CHART_COLORS: [&str; 8] = [
    "ambre", "orange", "rouge", "vert", "cyan", "bleu", "violet", "rose",
];
/// Les durées d'une courbe, en heures : 6 h, 24 h, 7 jours.
const CHART_HOURS: [u32; 3] = [6, 24, 168];
const MAX_STATES: usize = 10;
const MAX_TEXT_CHARS: usize = 60;
const MAX_PAGES: usize = 8;
const MAX_PAGE_ID_CHARS: usize = 30;
const MAX_PAGE_NAME_CHARS: usize = 30;
/// Les temps qu'il peut faire (les conditions de la météo), en plus du
/// soleil, qui est le fond de jour (`background`) ou de nuit (`night`).
const WEATHERS: [&str; 10] = [
    "eclaircies",
    "nuageux",
    "brouillard",
    "vent",
    "pluie",
    "averses",
    "orage",
    "grele",
    "neige",
    "pluie-neige",
];

/// Une ambiance : le moment, puis le temps (`jour-pluie`, `nuit-pluie-neige`).
fn is_ambiance(key: &str) -> bool {
    key.split_once('-').is_some_and(|(moment, weather)| {
        (moment == "jour" || moment == "nuit") && WEATHERS.contains(&weather)
    })
}

/// Les onglets du tableau de bord (Maison, Garage, Piscine…), chacun avec
/// son dessin. Le premier est celui qui s'ouvre.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Drawing {
    pages: Vec<Page>,
}

impl Default for Drawing {
    fn default() -> Self {
        Self {
            pages: vec![Page::home()],
        }
    }
}

/// Un onglet : son nom, les pièces qu'il montre, et l'assemblage de son
/// dessin.
///
/// Concept Rust : le même type sert à **lire** la demande (`Deserialize`,
/// avec `deny_unknown_fields`) et à **réécrire** ce qui est rangé
/// (`Serialize`) : seul ce qui a été vérifié est gardé.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Page {
    /// Identifiant court (`maison`, `garage`), gardé par chaque écran pour
    /// rouvrir le même onglet.
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    /// Les pièces montrées (noms de `areas.yaml`) ; vide : toutes.
    #[serde(default)]
    areas: Vec<String>,
    /// Le fond (de jour, ou toujours s'il n'y a pas de fond de nuit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background: Option<String>,
    /// Le fond de nuit (quand le soleil est couché).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    night: Option<String>,
    #[serde(default)]
    layers: Vec<Layer>,
    #[serde(default)]
    labels: Vec<Label>,
    /// Les fonds d'ambiance : `"jour-pluie": "maison_pluie"`… Il en manque ?
    /// Le plus proche s'affiche (nuit-pluie → nuit → jour-pluie → jour).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ambiances: BTreeMap<String, String>,
    /// Une ambiance imposée (pour essayer) au lieu de suivre la météo et le
    /// soleil : `jour`, `nuit`, ou une des ambiances.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    forced: Option<String>,
    /// Les boutons rapides sous le dessin (porche, parking, garage…).
    #[serde(default)]
    actions: Vec<String>,
    /// Les courbes de la colonne de gauche (la puissance du NAS sur 24 h…).
    #[serde(default)]
    charts: Vec<Chart>,
}

/// Une courbe : un capteur, sur une durée, dans une couleur.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Chart {
    entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    color: String,
    hours: u32,
}

impl Page {
    /// L'onglet « Maison », celui d'avant les onglets.
    fn home() -> Self {
        Self {
            id: "maison".into(),
            name: "Maison".into(),
            ..Self::default()
        }
    }
}

/// Un calque : visible quand l'appareil est dans l'un des états (ou dans
/// aucun, avec `not`), et seulement le jour ou la nuit si on le demande.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    image: String,
    entity_id: String,
    states: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    not: bool,
    #[serde(default)]
    when: When,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum When {
    #[default]
    Always,
    Day,
    Night,
}

/// Une valeur posée sur le dessin, à `x` % de la largeur et `y` % de la
/// hauteur.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Label {
    entity_id: String,
    x: f64,
    y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// Le type d'une image, d'après ses premiers octets (« nombre magique »).
fn image_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Un nom d'image : minuscules, chiffres, `_` et `-` (il finit dans une
/// adresse).
fn check_name(name: &str) -> Result<(), ApiError> {
    let valid = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "nom d'image « {name} » : 1 à {MAX_NAME_CHARS} caractères parmi a-z, 0-9, _ et -"
        )))
    }
}

fn check_text(what: &str, text: &str) -> Result<(), ApiError> {
    if text.is_empty()
        || text.chars().count() > MAX_TEXT_CHARS
        || text.chars().any(char::is_control)
    {
        return Err(ApiError::BadRequest(format!(
            "{what} : 1 à {MAX_TEXT_CHARS} caractères, sans retour à la ligne"
        )));
    }
    Ok(())
}

fn check_entity(entity_id: &str) -> Result<EntityId, ApiError> {
    entity_id
        .parse()
        .map_err(|error: ha_core::CoreError| ApiError::BadRequest(error.to_string()))
}

/// Vérifie les onglets : 1 à 8, des identifiants courts et uniques, des
/// noms, des pièces qui existent, et le dessin de chacun.
fn check_drawing(
    drawing: &Drawing,
    images: &HashSet<String>,
    areas: &HashSet<&str>,
) -> Result<(), ApiError> {
    if drawing.pages.is_empty() || drawing.pages.len() > MAX_PAGES {
        return Err(ApiError::BadRequest(format!("1 à {MAX_PAGES} onglets")));
    }
    let mut ids = HashSet::new();
    for page in &drawing.pages {
        let valid_id = !page.id.is_empty()
            && page.id.chars().count() <= MAX_PAGE_ID_CHARS
            && page
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid_id {
            return Err(ApiError::BadRequest(format!(
                "onglet « {} » : identifiant de 1 à {MAX_PAGE_ID_CHARS} caractères parmi a-z, 0-9 et -",
                page.id
            )));
        }
        if !ids.insert(page.id.as_str()) {
            return Err(ApiError::BadRequest(format!(
                "onglet « {} » en double",
                page.id
            )));
        }
        if page.name.chars().count() > MAX_PAGE_NAME_CHARS {
            return Err(ApiError::BadRequest(format!(
                "onglet « {} » : nom de {MAX_PAGE_NAME_CHARS} caractères au plus",
                page.id
            )));
        }
        check_text("nom d'onglet", &page.name)?;
        for area in &page.areas {
            if !areas.contains(area.as_str()) {
                return Err(ApiError::BadRequest(format!(
                    "onglet « {} » : pièce « {area} » introuvable dans areas.yaml",
                    page.name
                )));
            }
        }
        check_page(page, images).map_err(|error| match error {
            ApiError::BadRequest(message) => {
                ApiError::BadRequest(format!("onglet « {} » : {message}", page.name))
            }
            other => other,
        })?;
    }
    Ok(())
}

/// Vérifie le dessin d'un onglet : des images qui existent, des appareils
/// bien écrits, des positions sur le dessin.
fn check_page(config: &Page, images: &HashSet<String>) -> Result<(), ApiError> {
    let known = |name: &String| -> Result<(), ApiError> {
        if images.contains(name) {
            Ok(())
        } else {
            Err(ApiError::BadRequest(format!(
                "image « {name} » introuvable"
            )))
        }
    };
    if config.night.is_some() && config.background.is_none() {
        return Err(ApiError::BadRequest(
            "un fond de nuit demande aussi un fond de jour".into(),
        ));
    }
    config
        .background
        .iter()
        .chain(&config.night)
        .chain(config.ambiances.values())
        .try_for_each(known)?;
    if !config.ambiances.is_empty() && config.background.is_none() {
        return Err(ApiError::BadRequest(
            "les fonds d'ambiance demandent aussi un fond de jour".into(),
        ));
    }
    if let Some(key) = config.ambiances.keys().find(|key| !is_ambiance(key)) {
        return Err(ApiError::BadRequest(format!(
            "ambiance « {key} » inconnue : jour- ou nuit- puis {}",
            WEATHERS.join(", ")
        )));
    }
    if let Some(forced) = &config.forced
        && forced != "jour"
        && forced != "nuit"
        && !is_ambiance(forced)
    {
        return Err(ApiError::BadRequest(format!(
            "ambiance imposée « {forced} » inconnue"
        )));
    }
    if config.layers.len() > MAX_LAYERS {
        return Err(ApiError::BadRequest(format!(
            "{MAX_LAYERS} calques au plus"
        )));
    }
    for (index, layer) in config.layers.iter().enumerate() {
        let n = index + 1;
        known(&layer.image)?;
        check_entity(&layer.entity_id)?;
        if layer.states.is_empty() || layer.states.len() > MAX_STATES {
            return Err(ApiError::BadRequest(format!(
                "calque {n} : 1 à {MAX_STATES} états"
            )));
        }
        for value in &layer.states {
            check_text(&format!("calque {n}, état"), value)?;
        }
    }
    if config.actions.len() > MAX_ACTIONS {
        return Err(ApiError::BadRequest(format!(
            "{MAX_ACTIONS} boutons au plus"
        )));
    }
    let mut seen = HashSet::new();
    for entity_id in &config.actions {
        check_entity(entity_id)?;
        if !seen.insert(entity_id) {
            return Err(ApiError::BadRequest(format!(
                "bouton « {entity_id} » en double"
            )));
        }
    }
    if config.charts.len() > MAX_CHARTS {
        return Err(ApiError::BadRequest(format!(
            "{MAX_CHARTS} courbes au plus"
        )));
    }
    for (index, chart) in config.charts.iter().enumerate() {
        let n = index + 1;
        check_entity(&chart.entity_id)?;
        if !CHART_COLORS.contains(&chart.color.as_str()) {
            return Err(ApiError::BadRequest(format!(
                "courbe {n} : couleur parmi {}",
                CHART_COLORS.join(", ")
            )));
        }
        if !CHART_HOURS.contains(&chart.hours) {
            return Err(ApiError::BadRequest(format!(
                "courbe {n} : 6, 24 ou 168 heures"
            )));
        }
        if let Some(name) = &chart.name {
            check_text(&format!("courbe {n}, nom"), name)?;
        }
    }
    if config.labels.len() > MAX_LABELS {
        return Err(ApiError::BadRequest(format!(
            "{MAX_LABELS} valeurs au plus"
        )));
    }
    for (index, label) in config.labels.iter().enumerate() {
        let n = index + 1;
        check_entity(&label.entity_id)?;
        let inside = |v: f64| v.is_finite() && (0.0..=100.0).contains(&v);
        if !inside(label.x) || !inside(label.y) {
            return Err(ApiError::BadRequest(format!(
                "valeur {n} : position entre 0 et 100 %"
            )));
        }
        if let Some(name) = &label.name {
            check_text(&format!("valeur {n}, nom"), name)?;
        }
    }
    Ok(())
}

/// Arrondit les positions au dixième de pour-cent : assez précis, et rien
/// de plus n'est gardé.
fn tidy(mut drawing: Drawing) -> Drawing {
    for label in drawing.pages.iter_mut().flat_map(|page| &mut page.labels) {
        label.x = (label.x * 10.0).round() / 10.0;
        label.y = (label.y * 10.0).round() / 10.0;
    }
    drawing
}

/// Relit ce qui est rangé. Avant les onglets, c'était le dessin d'une seule
/// page : il devient l'onglet « Maison ». Un contenu illisible vaut « pas de
/// dessin ».
fn parse_stored(text: &str) -> Drawing {
    if let Ok(drawing) = serde_json::from_str::<Drawing>(text) {
        return drawing;
    }
    match serde_json::from_str::<Page>(text) {
        Ok(page) => Drawing {
            pages: vec![Page {
                id: "maison".into(),
                name: "Maison".into(),
                ..page
            }],
        },
        Err(_) => Drawing::default(),
    }
}

/// Ce qu'un compte peut voir de l'assemblage : un invité ne voit ni les
/// calques ni les valeurs des appareils qui lui sont cachés.
fn visible_to(state: &ApiState, user: &User, mut drawing: Drawing) -> Drawing {
    let can_see = |entity_id: &str| {
        entity_id.parse::<EntityId>().is_ok_and(|id| {
            let current = state.core.states.get(&id);
            roles::can_see(user, &id, current.as_ref())
        })
    };
    for page in &mut drawing.pages {
        page.layers.retain(|layer| can_see(&layer.entity_id));
        page.labels.retain(|label| can_see(&label.entity_id));
        page.actions.retain(|entity_id| can_see(entity_id));
        page.charts.retain(|chart| can_see(&chart.entity_id));
    }
    drawing
}

async fn stored_config(state: &ApiState) -> Result<Drawing, ApiError> {
    let stored = state
        .store
        .home_drawing()
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(stored.map(|text| parse_stored(&text)).unwrap_or_default())
}

async fn image_names(state: &ApiState) -> Result<Vec<ha_storage::HomeImageInfo>, ApiError> {
    state
        .store
        .home_images()
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))
}

/// `GET /api/home/drawing` : l'assemblage, et les images disponibles.
pub(crate) async fn get(
    State(state): State<Arc<ApiState>>,
    auth: Authenticated,
) -> Result<Json<Value>, ApiError> {
    let config = visible_to(&state, &auth.user, stored_config(&state).await?);
    let images: Vec<Value> = image_names(&state)
        .await?
        .into_iter()
        .map(|image| {
            json!({
                "name": image.name,
                "content_type": image.content_type,
                "size": image.size,
                "updated_at": format_timestamp(image.updated_at),
            })
        })
        .collect();
    Ok(Json(json!({
        "config": config,
        "images": images,
        "editable": auth.user.can(Permission::EditDashboard),
    })))
}

/// `PUT /api/home/drawing` : change l'assemblage.
pub(crate) async fn put(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    roles::require(&auth.user, Permission::EditDashboard)?;
    let config = tidy(parse_json::<Drawing>(&body)?);
    let images: HashSet<String> = image_names(&state)
        .await?
        .into_iter()
        .map(|image| image.name)
        .collect();
    let areas: HashSet<&str> = state.areas.iter().map(|area| area.name.as_str()).collect();
    check_drawing(&config, &images, &areas)?;
    let text =
        serde_json::to_string(&config).map_err(|error| ApiError::Internal(error.to_string()))?;
    let username = &auth.user.username;
    state
        .store
        .save_home_drawing(&text, username)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    tracing::info!("dessin de la maison réglé par {username}");
    let names: Vec<&str> = config.pages.iter().map(|page| page.name.as_str()).collect();
    let detail = format!("onglets : {}", names.join(", "));
    audit(&state, username, "drawing_set", None, Some(&detail), client).await;
    Ok(Json(json!({ "config": config })))
}

/// `GET /api/home/images/{name}` : une image, pour tout compte connecté.
pub(crate) async fn get_image(
    State(state): State<Arc<ApiState>>,
    _auth: Authenticated,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    check_name(&name)?;
    let image = state
        .store
        .home_image(&name)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .ok_or(ApiError::NotFound)?;
    // Relue dans la base : on revérifie quand même le type.
    let content_type = image_type(&image.data).ok_or(ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-cache"),
            ),
        ],
        image.data,
    )
        .into_response())
}

/// `PUT /api/home/images/{name}` : le corps est l'image elle-même.
pub(crate) async fn put_image(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    roles::require(&auth.user, Permission::EditDashboard)?;
    check_name(&name)?;
    if body.len() > MAX_IMAGE_SIZE {
        return Err(ApiError::BadRequest(
            "image trop grosse (3 Mio au plus)".into(),
        ));
    }
    let content_type = image_type(&body)
        .ok_or_else(|| ApiError::BadRequest("ce n'est pas une image PNG, JPEG ou WebP".into()))?;
    let images = image_names(&state).await?;
    if images.len() >= MAX_IMAGES && !images.iter().any(|image| image.name == name) {
        return Err(ApiError::BadRequest(format!(
            "{MAX_IMAGES} images au plus : retire-en une d'abord"
        )));
    }
    let username = &auth.user.username;
    state
        .store
        .save_home_image(&name, content_type, &body, username)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    tracing::info!("image « {name} » du dessin enregistrée par {username}");
    let detail = format!("{content_type}, {} Kio", body.len().div_ceil(1024));
    audit(
        &state,
        username,
        "drawing_image",
        Some(&name),
        Some(&detail),
        client,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/home/images/{name}` : refusé tant que l'assemblage s'en sert.
pub(crate) async fn delete_image(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    roles::require(&auth.user, Permission::EditDashboard)?;
    check_name(&name)?;
    let config = stored_config(&state).await?;
    let used = config.pages.iter().any(|page| {
        page.background.as_ref() == Some(&name)
            || page.night.as_ref() == Some(&name)
            || page.layers.iter().any(|layer| layer.image == name)
            || page.ambiances.values().any(|image| *image == name)
    });
    if used {
        return Err(ApiError::BadRequest(format!(
            "l'image « {name} » sert encore au dessin : retire-la d'abord du fond ou des calques"
        )));
    }
    let removed = state
        .store
        .delete_home_image(&name)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    let username = &auth.user.username;
    tracing::info!("image « {name} » du dessin retirée par {username}");
    audit(
        &state,
        username,
        "drawing_image_delete",
        Some(&name),
        None,
        client,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_png_jpeg_and_webp_are_images() {
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(image_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(image_type(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(
            image_type(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"),
            None
        );
        assert_eq!(image_type(b"<html><script>"), None);
        assert_eq!(image_type(b"RIFF\x10\0\0\0WAVE"), None);
        assert_eq!(image_type(b""), None);
    }

    #[test]
    fn image_names_are_short_and_plain() {
        assert!(check_name("garage_ouvert-2").is_ok());
        for bad in ["", "Jour", "../ha.key", "a b", "é", &"x".repeat(41)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    fn config(json: &str) -> Page {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn the_drawing_is_checked() {
        let images: HashSet<String> = ["jour".into(), "garage".into()].into();
        let good = config(
            r#"{"background":"jour","layers":[{"image":"garage","entity_id":"cover.garage",
                "states":["open","opening"],"when":"day"}],
                "labels":[{"entity_id":"sensor.dehors","x":90,"y":8.25,"name":"Dehors"}]}"#,
        );
        assert!(check_page(&good, &images).is_ok());
        let rainy = config(
            r#"{"background":"jour","ambiances":{"jour-pluie":"garage","nuit-pluie-neige":"jour","jour-orage":"jour"},"forced":"nuit-pluie-neige"}"#,
        );
        assert!(check_page(&rainy, &images).is_ok());
        let charts = config(
            r#"{"charts":[{"entity_id":"sensor.nas","name":"NAS","color":"violet","hours":24},
                {"entity_id":"sensor.nvr","color":"ambre","hours":6}]}"#,
        );
        assert!(check_page(&charts, &images).is_ok());
        let tidied = tidy(Drawing { pages: vec![good] });
        assert!((tidied.pages[0].labels[0].y - 8.3).abs() < 1e-9);

        for bad in [
            r#"{"background":"nuit"}"#,
            r#"{"night":"jour"}"#,
            r#"{"layers":[{"image":"garage","entity_id":"cover.garage","states":[]}]}"#,
            r#"{"layers":[{"image":"garage","entity_id":"pas un appareil","states":["on"]}]}"#,
            r#"{"labels":[{"entity_id":"sensor.a","x":101,"y":0}]}"#,
            r#"{"labels":[{"entity_id":"sensor.a","x":1,"y":1,"name":"a\nb"}]}"#,
            r#"{"actions":["light.porche","light.porche"]}"#,
            r#"{"actions":["pas un appareil"]}"#,
            r#"{"background":"jour","ambiances":{"jour-tempete":"garage"}}"#,
            r#"{"background":"jour","ambiances":{"midi-pluie":"garage"}}"#,
            r#"{"ambiances":{"jour-pluie":"garage"}}"#,
            r#"{"background":"jour","ambiances":{"jour-pluie":"absente"}}"#,
            r#"{"forced":"tempete"}"#,
            r#"{"charts":[{"entity_id":"sensor.nas","color":"fuchsia","hours":24}]}"#,
            r#"{"charts":[{"entity_id":"sensor.nas","color":"vert","hours":12}]}"#,
            r#"{"charts":[{"entity_id":"pas un capteur","color":"vert","hours":24}]}"#,
        ] {
            assert!(check_page(&config(bad), &images).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<Page>(r#"{"script":"x"}"#).is_err());
    }

    #[test]
    fn pages_are_checked() {
        let images: HashSet<String> = HashSet::new();
        let areas: HashSet<&str> = ["Garage", "Salon"].into();
        let drawing = |json: &str| serde_json::from_str::<Drawing>(json).unwrap();
        let good = drawing(
            r#"{"pages":[{"id":"maison","name":"Maison"},
                {"id":"garage","name":"Garage","areas":["Garage"]}]}"#,
        );
        assert!(check_drawing(&good, &images, &areas).is_ok());
        for bad in [
            r#"{"pages":[]}"#,
            r#"{"pages":[{"id":"a","name":"A"},{"id":"a","name":"B"}]}"#,
            r#"{"pages":[{"id":"Maison","name":"Maison"}]}"#,
            r#"{"pages":[{"id":"m","name":""}]}"#,
            r#"{"pages":[{"id":"m","name":"M","areas":["Grenier"]}]}"#,
        ] {
            assert!(
                check_drawing(&drawing(bad), &images, &areas).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_drawing_from_before_the_pages_becomes_the_home_page() {
        let old = r#"{"background":"jour","layers":[],"labels":[],"actions":["light.porche"]}"#;
        let drawing = parse_stored(old);
        assert_eq!(drawing.pages.len(), 1);
        assert_eq!(drawing.pages[0].id, "maison");
        assert_eq!(drawing.pages[0].name, "Maison");
        assert_eq!(drawing.pages[0].background.as_deref(), Some("jour"));
        assert_eq!(drawing.pages[0].actions, ["light.porche"]);
        assert_eq!(parse_stored("pas du json").pages[0].id, "maison");
    }
}
