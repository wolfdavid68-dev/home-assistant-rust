//! Le mot d'appel « Maison » (étape 97) : la tablette écoute en continu,
//! découpe ce qu'elle entend en **morceaux de parole** (0,4 à 6 s) et les
//! envoie ici. Le serveur les fait écouter (Parakeet, sur le mini-PC) et
//! regarde si la phrase **commence par le mot d'appel** :
//!
//! - pas de mot : la réponse est `{"wake": false}`, et rien n'est gardé ni
//!   écrit dans les journaux (ni le son, ni le texte) ;
//! - « Maison » seul : `{"wake": true}`, la tablette ouvre le micro comme
//!   avec le bouton ;
//! - « Maison, allume la cuisine » : la commande est comprise et obéie
//!   comme une phrase de `/api/conversation/audio`, avec les mêmes droits.
//!
//! Les garde-fous : une vraie phrase passe toujours avant (un morceau est
//! abandonné si l'écoute est occupée, sans attendre), un nombre de morceaux
//! limité par appareil, des morceaux courts, et seulement le service
//! d'écoute (qui garde le modèle en mémoire).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use ha_stt::SttError;
use serde_json::{Value, json};

use crate::ApiState;
use crate::assist::converse_in;
use crate::auth::Authenticated;
use crate::error::ApiError;

/// Un morceau dure au plus ce temps (la tablette en envoie 6 s au plus).
pub(crate) const MAX_MILLIS: u32 = 6_500;
/// La taille d'un morceau de [`MAX_MILLIS`], avec de la marge pour l'en-tête.
pub(crate) const MAX_BYTES: usize = (ha_stt::RATE * 2 * MAX_MILLIS / 1000) as usize + 4096;
/// Morceaux au plus par minute et par appareil (un toutes les 2 s en
/// moyenne : une pièce bavarde, pas un flot continu).
pub(crate) const MAX_PER_MINUTE: usize = 30;
/// Le mot d'appel par défaut (réglage `HA_MOT_APPEL`).
pub(crate) const DEFAULT_WORD: &str = "Maison";
/// Au-delà de ce nombre d'appareils suivis, on oublie ceux d'il y a plus
/// d'une minute.
const MAX_DEVICES: usize = 256;
/// Les mots qu'on peut dire avant le mot d'appel (« Ok Maison », « Dis
/// Maison »).
const BEFORE: [&str; 10] = [
    "ok", "okay", "dis", "di", "hey", "he", "eh", "o", "oh", "allo",
];

/// Le mot d'appel, son réglage et la limite des morceaux.
#[derive(Debug, Default)]
pub(crate) struct Wake {
    word: OnceLock<WakeWord>,
    /// Empreinte du jeton de l'appareil → ses derniers morceaux.
    recent: Mutex<HashMap<[u8; 32], VecDeque<Instant>>>,
}

impl Wake {
    /// Le mot d'appel réglé, ou « Maison ».
    pub(crate) fn word(&self) -> &WakeWord {
        self.word.get_or_init(WakeWord::default)
    }

    /// Règle le mot d'appel (au démarrage).
    pub(crate) fn set_word(&self, setting: &str) -> Result<(), String> {
        let word = WakeWord::new(setting)?;
        self.word
            .set(word)
            .map_err(|_| "le mot d'appel est déjà réglé".to_owned())
    }

    /// Ce morceau de l'appareil `device` passe-t-il ? Sinon, le temps à
    /// attendre.
    fn allow(&self, device: [u8; 32]) -> Result<(), Duration> {
        let now = Instant::now();
        let minute = Duration::from_mins(1);
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        if recent.len() >= MAX_DEVICES {
            recent.retain(|_, times| times.back().is_some_and(|t| now - *t < minute));
        }
        let times = recent.entry(device).or_default();
        while times.front().is_some_and(|t| now - *t >= minute) {
            times.pop_front();
        }
        if times.len() >= MAX_PER_MINUTE {
            let oldest = times.front().copied().unwrap_or(now);
            return Err(minute
                .saturating_sub(now - oldest)
                .max(Duration::from_secs(1)));
        }
        times.push_back(now);
        Ok(())
    }
}

/// Un mot d'appel : ses mots tels qu'on les a écrits, et leur prononciation
/// simplifiée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WakeWord {
    shown: String,
    keys: Vec<String>,
}

impl Default for WakeWord {
    fn default() -> Self {
        Self {
            shown: DEFAULT_WORD.to_owned(),
            keys: vec![sound(&ha_assist::normalize(DEFAULT_WORD))],
        }
    }
}

/// Ce qu'a donné un morceau.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Heard<'a> {
    /// Pas de mot d'appel au début.
    Nothing,
    /// Le mot d'appel seul.
    Alone,
    /// Le mot d'appel puis une commande.
    Command(&'a str),
}

impl WakeWord {
    /// D'après le réglage `HA_MOT_APPEL` : 1 à 3 mots, des lettres.
    pub(crate) fn new(setting: &str) -> Result<Self, String> {
        let shown = setting.split_whitespace().collect::<Vec<_>>().join(" ");
        let keys: Vec<String> = words(&shown)
            .map(|(_, word)| sound(&ha_assist::normalize(word)))
            .collect();
        let letters_only = shown
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\''));
        if keys.is_empty()
            || keys.len() > 3
            || shown.chars().count() > 40
            || !letters_only
            || keys.iter().any(|key| key.len() < 2)
        {
            return Err(format!(
                "HA_MOT_APPEL : « {shown} » ne convient pas (1 à 3 mots, des lettres seulement)"
            ));
        }
        Ok(Self { shown, keys })
    }

    /// Le mot tel qu'on l'a écrit (« Maison »), pour la tablette.
    pub(crate) fn shown(&self) -> &str {
        &self.shown
    }

    /// La phrase entendue commence-t-elle par le mot d'appel ? Tolérant aux
    /// majuscules, aux accents, à la ponctuation et aux petites variantes
    /// d'écoute (« Maisons », « Mézon »), avec « Ok » ou « Dis » devant.
    pub(crate) fn find<'a>(&self, text: &'a str) -> Heard<'a> {
        let mut said = words(text).peekable();
        // « Ok », « Dis »… devant : deux au plus.
        for _ in 0..2 {
            let Some((_, word)) = said.peek() else {
                break;
            };
            let plain = ha_assist::normalize(word);
            // (Sauf si le mot d'appel lui-même commence par l'un d'eux.)
            if BEFORE.contains(&plain.as_str()) && self.keys.first() != Some(&sound(&plain)) {
                said.next();
            }
        }
        let mut end = 0;
        for key in &self.keys {
            let Some((at, word)) = said.next() else {
                return Heard::Nothing;
            };
            if !close(&sound(&ha_assist::normalize(word)), key) {
                return Heard::Nothing;
            }
            end = at + word.len();
        }
        let rest = text[end..].trim_start_matches(|c: char| !c.is_alphanumeric());
        if rest.is_empty() {
            Heard::Alone
        } else {
            Heard::Command(rest.trim_end())
        }
    }
}

/// Les mots d'un texte, avec leur position : ce qui est entre les espaces,
/// la ponctuation, les apostrophes et les traits d'union.
fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = None;
    let mut out = Vec::new();
    for (at, c) in text.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                out.push((from, &text[from..at]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        out.push((from, &text[from..]));
    }
    out.into_iter()
}

/// La prononciation simplifiée d'un mot déjà en minuscules sans accents :
/// « ai » et « ei » comme « e », « z » comme « s », sans « s » ni « x »
/// muets à la fin. « maison », « maisons », « mezon » → « meson ».
fn sound(word: &str) -> String {
    let mut key = word
        .replace(' ', "")
        .replace("ai", "e")
        .replace("ei", "e")
        .replace('z', "s")
        .replace("ss", "s")
        .replace('y', "i");
    if key.len() > 3 && (key.ends_with('s') || key.ends_with('x')) {
        key.pop();
    }
    key
}

/// Deux prononciations proches : égales, ou (pour un mot de 5 lettres et
/// plus) une seule lettre de différence, sans toucher à la première
/// (« raison » et « saison » ne réveillent pas « maison »).
fn close(heard: &str, key: &str) -> bool {
    if heard == key {
        return true;
    }
    key.len() >= 5 && heard.chars().next() == key.chars().next() && one_edit(heard, key)
}

/// Une seule lettre ajoutée, retirée ou changée.
fn one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let prefix = short.iter().zip(&long).take_while(|(x, y)| x == y).count();
    if short.len() == long.len() {
        short[prefix + 1..] == long[prefix + 1..]
    } else {
        short[prefix..] == long[prefix + 1..]
    }
}

/// `POST /api/conversation/wake`
pub(crate) async fn wake(
    State(state): State<Arc<ApiState>>,
    ConnectInfo(client): ConnectInfo<SocketAddr>,
    auth: Authenticated,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let stt = state
        .listening
        .get()
        .filter(|stt| stt.keeps_model())
        .ok_or_else(|| {
            ApiError::Unavailable(
                "le mot d'appel demande le service d'écoute (voir docs/mot-appel.md)".into(),
            )
        })?;
    // Chaque appareil a son jeton : la limite est par appareil.
    if let Err(retry_after) = state.wake.allow(ha_auth::token::hash_token(&auth.token)) {
        return Err(ApiError::TooManyRequests { retry_after });
    }
    let samples =
        ha_stt::check_wav(&body).map_err(|error| ApiError::BadRequest(error.to_string()))?;
    if samples.len() > (ha_stt::RATE * 2 * MAX_MILLIS / 1000) as usize {
        return Err(ApiError::BadRequest(format!(
            "morceau trop long ({} s au plus)",
            MAX_MILLIS / 1000
        )));
    }
    let text = match stt.transcribe_if_free(&body).await {
        Ok(text) => text,
        Err(SttError::Silence) => return Ok(Json(json!({"wake": false}))),
        // Une vraie phrase est écoutée, ou attend : le morceau est abandonné.
        Err(SttError::Busy) => return Ok(Json(json!({"wake": false, "busy": true}))),
        Err(SttError::BadAudio(why)) => {
            return Err(ApiError::BadRequest(format!("son refusé : {why}")));
        }
        Err(error) => {
            tracing::warn!("mot d'appel : {error}");
            return Err(ApiError::Unavailable("l'écoute a échoué".into()));
        }
    };
    match state.wake.word().find(&text) {
        // Ni le texte ni le son ne sont gardés.
        Heard::Nothing => Ok(Json(json!({"wake": false}))),
        Heard::Alone => Ok(Json(json!({"wake": true}))),
        Heard::Command(command) => {
            let turn = converse_in(&state, &auth, client, command, None).await;
            let mut reply = turn.json();
            reply["wake"] = json!(true);
            reply["text"] = json!(command);
            Ok(Json(reply))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn the_wake_word_starts_the_sentence() {
        let maison = WakeWord::default();
        assert_eq!(
            maison.find("Maison, allume la lumière du salon."),
            Heard::Command("allume la lumière du salon.")
        );
        assert_eq!(
            maison.find("maison allume la cuisine"),
            Heard::Command("allume la cuisine")
        );
        assert_eq!(maison.find("Maison."), Heard::Alone);
        assert_eq!(maison.find("MAISON !"), Heard::Alone);
        assert_eq!(maison.find(" maison ? "), Heard::Alone);
        // « Ok », « Dis » devant.
        assert_eq!(
            maison.find("Ok Maison, ferme le volet."),
            Heard::Command("ferme le volet.")
        );
        assert_eq!(maison.find("Dis, maison…"), Heard::Alone);
    }

    #[test]
    fn small_listening_variants_are_tolerated() {
        let maison = WakeWord::default();
        for heard in ["Maisons", "Mézon", "Maizon", "Méson", "Mason", "Maisonn"] {
            assert_eq!(maison.find(heard), Heard::Alone, "{heard}");
        }
    }

    #[test]
    fn other_sentences_do_not_wake() {
        let maison = WakeWord::default();
        for heard in [
            "",
            "Il fait beau aujourd'hui.",
            "Ma maison est grande.",
            "À la maison, on mange tôt.",
            "Raison de plus.",
            "Saison des pluies.",
            "Mais on y va ?",
            "Maçon",
            "Ok",
        ] {
            assert_eq!(maison.find(heard), Heard::Nothing, "{heard}");
        }
    }

    #[test]
    fn the_wake_word_can_be_chosen() {
        let jarvis = WakeWord::new("  Jarvis ").unwrap();
        assert_eq!(jarvis.shown(), "Jarvis");
        assert_eq!(
            jarvis.find("Jarvis, éteins tout."),
            Heard::Command("éteins tout.")
        );
        assert_eq!(jarvis.find("Maison, éteins tout."), Heard::Nothing);
        let two = WakeWord::new("Petite maison").unwrap();
        assert_eq!(two.find("Petite maison !"), Heard::Alone);
        assert_eq!(two.find("Maison !"), Heard::Nothing);
        for wrong in ["", "   ", "Maison2", "a b c d", "x", "<script>"] {
            assert!(WakeWord::new(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn each_device_has_its_own_limit() {
        let wake = Wake::default();
        for _ in 0..MAX_PER_MINUTE {
            assert!(wake.allow([1; 32]).is_ok());
        }
        let wait = wake.allow([1; 32]).unwrap_err();
        assert!(wait > Duration::from_secs(50), "{wait:?}");
        assert!(wake.allow([2; 32]).is_ok());
    }
}
