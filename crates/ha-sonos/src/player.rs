//! Une enceinte comme entité `media_player`, comme dans Home Assistant :
//! son état (`playing`, `paused`, `idle`) et ce qu'elle joue.

use ha_core::Attributes;
use serde_json::{Value, json};

use crate::api::{Media, Position};
use crate::xml;

/// Ce qu'on a lu d'une enceinte à un instant donné.
#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub name: String,
    /// État de lecture du **coordinateur** de son groupe.
    pub transport: String,
    pub position: Position,
    pub media: Media,
    pub volume: u8,
    pub muted: bool,
    /// Les autres enceintes du groupe (leurs entités), s'il y en a.
    pub group: Vec<String>,
    /// L'adresse du coordinateur du groupe (`192.168.1.30:1400`).
    pub coordinator: String,
    /// Le mode de lecture du groupe (`SHUFFLE`, `REPEAT_ALL`…), s'il a pu
    /// être lu.
    pub play_mode: Option<String>,
}

/// Une position qui s'écarte de moins que ça de celle attendue n'est pas
/// republiée : le tableau de bord fait avancer la barre tout seul.
const DRIFT: u64 = 4;

/// Ce qui est en cours : titre, artiste, album, station.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub channel: Option<String>,
}

/// `PLAYING` → `playing`, comme dans Home Assistant.
#[must_use]
pub fn state_name(transport: &str) -> &'static str {
    match transport {
        "PLAYING" | "TRANSITIONING" => "playing",
        "PAUSED_PLAYBACK" => "paused",
        _ => "idle",
    }
}

/// L'état et les attributs de l'entité.
#[must_use]
pub fn entity(reading: &Reading) -> (&'static str, Attributes) {
    let state = state_name(&reading.transport);
    let mut attributes = Attributes::new();
    attributes.insert("friendly_name".into(), json!(reading.name));
    attributes.insert("device_class".into(), json!("speaker"));
    attributes.insert(
        "volume_level".into(),
        json!(f64::from(reading.volume) / 100.0),
    );
    attributes.insert("is_volume_muted".into(), json!(reading.muted));
    if state != "idle" {
        let details = details(&reading.position, &reading.media);
        for (key, value) in [
            ("media_title", details.title),
            ("media_artist", details.artist),
            ("media_album_name", details.album),
            ("media_channel", details.channel),
        ] {
            if let Some(value) = value {
                attributes.insert(key.into(), json!(value));
            }
        }
    }
    if state != "idle" {
        if let Some(duration) = seconds(&reading.position.duration).filter(|d| *d > 0) {
            attributes.insert("media_duration".into(), json!(duration));
        }
        // Aléatoire et répétition n'ont de sens que pour la file d'attente.
        if reading.media.uri.starts_with("x-rincon-queue:")
            && let Some((shuffle, repeat)) = reading.play_mode.as_deref().and_then(play_mode)
        {
            attributes.insert("shuffle".into(), json!(shuffle));
            attributes.insert("repeat".into(), json!(repeat));
        }
    }
    if !reading.group.is_empty() {
        attributes.insert("group_members".into(), json!(reading.group));
    }
    (state, attributes)
}

/// Ajoute `media_position` et `media_position_updated_at`, comme Home
/// Assistant : la position à un instant donné. Tant que le morceau avance
/// comme prévu, on garde les valeurs précédentes (`previous`) : l'état ne
/// change pas toutes les 3 s, et le tableau de bord calcule le reste.
pub fn track_position(
    attributes: &mut Attributes,
    state: &str,
    reading: &Reading,
    previous: Option<(&str, &Attributes)>,
    now: jiff::Timestamp,
) {
    if !attributes.contains_key("media_duration") {
        return;
    }
    let Some(position) = seconds(&reading.position.rel_time) else {
        return;
    };
    let kept = previous.and_then(|(old_state, old)| {
        let same = old_state == state
            && ["media_title", "media_artist", "media_duration"]
                .iter()
                .all(|key| old.get(*key) == attributes.get(*key));
        if !same {
            return None;
        }
        let old_position = old.get("media_position")?.as_u64()?;
        let at = old.get("media_position_updated_at")?.as_str()?;
        let since: jiff::Timestamp = at.parse().ok()?;
        let elapsed = if state == "playing" {
            u64::try_from(now.as_second() - since.as_second()).unwrap_or(0)
        } else {
            0
        };
        ((old_position + elapsed).abs_diff(position) < DRIFT).then(|| (old_position, at.to_owned()))
    });
    let (position, at) = kept.unwrap_or_else(|| {
        let now = jiff::Timestamp::from_second(now.as_second()).unwrap_or(now);
        (position, now.to_string())
    });
    attributes.insert("media_position".into(), json!(position));
    attributes.insert("media_position_updated_at".into(), Value::String(at));
}

/// `0:03:45` → 225 secondes. `None` si ce n'est pas une durée.
#[must_use]
pub fn seconds(text: &str) -> Option<u64> {
    let mut total: u64 = 0;
    let mut parts = 0;
    for part in text.trim().split(':') {
        let n: u64 = part.parse().ok()?;
        if parts > 0 && n >= 60 {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(n)?;
        parts += 1;
    }
    (parts == 3).then_some(total)
}

/// 225 → `0:03:45`, pour aller à un moment du morceau.
#[must_use]
pub fn hms(seconds: u64) -> String {
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Le mode Sonos en (aléatoire, répétition), comme Home Assistant :
/// répétition `off`, `all` ou `one`.
#[must_use]
pub fn play_mode(mode: &str) -> Option<(bool, &'static str)> {
    Some(match mode {
        "NORMAL" => (false, "off"),
        "REPEAT_ALL" => (false, "all"),
        "REPEAT_ONE" => (false, "one"),
        "SHUFFLE_NOREPEAT" => (true, "off"),
        "SHUFFLE" => (true, "all"),
        "SHUFFLE_REPEAT_ONE" => (true, "one"),
        _ => return None,
    })
}

/// L'inverse de [`play_mode`].
#[must_use]
pub fn mode_name(shuffle: bool, repeat: &str) -> Option<&'static str> {
    Some(match (shuffle, repeat) {
        (false, "off") => "NORMAL",
        (false, "all") => "REPEAT_ALL",
        (false, "one") => "REPEAT_ONE",
        (true, "off") => "SHUFFLE_NOREPEAT",
        (true, "all") => "SHUFFLE",
        (true, "one") => "SHUFFLE_REPEAT_ONE",
        _ => return None,
    })
}

/// Titre, artiste… d'après les métadonnées DIDL (du XML rangé dans la
/// réponse). Une radio donne « Artiste - Titre » dans `streamContent`, et
/// son nom dans les métadonnées de ce qui est chargé.
#[must_use]
pub fn details(position: &Position, media: &Media) -> Details {
    if media.uri.starts_with("x-sonos-htastream:") {
        return Details {
            title: Some("TV".into()),
            ..Details::default()
        };
    }
    if media.uri.starts_with("x-rincon-stream:") {
        return Details {
            title: Some("Entrée audio".into()),
            ..Details::default()
        };
    }
    let field = |xml: &str, name| {
        xml::element_text(xml, name)
            .map(|text| clean(&text))
            .filter(|text| !text.is_empty() && !looks_like_address(text))
    };
    let meta = position.metadata.as_str();
    let mut details = Details {
        title: field(meta, "title"),
        artist: field(meta, "creator"),
        album: field(meta, "album"),
        channel: None,
    };
    if let Some(stream) = field(meta, "streamContent") {
        match stream.split_once(" - ") {
            Some((artist, title)) => {
                details.artist = Some(artist.trim().to_owned());
                details.title = Some(title.trim().to_owned());
            }
            None => details.title = Some(stream),
        }
    }
    let station = field(&media.metadata, "title");
    let radio = media.uri.starts_with("x-rincon-mp3radio:")
        || media.uri.starts_with("x-sonosapi-stream:")
        || media.uri.starts_with("x-sonosapi-radio:")
        || media.uri.starts_with("aac:")
        || media.uri.starts_with("hls-radio:");
    if radio {
        details.channel.clone_from(&station);
        // Le titre d'une radio est souvent son nom : on ne le répète pas.
        if details.title == station {
            details.title = None;
        }
    }
    details
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Un « titre » qui n'est que l'adresse du flux.
fn looks_like_address(text: &str) -> bool {
    text.starts_with("x-") || text.starts_with("http://") || text.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn didl(inner: &str) -> String {
        format!("<DIDL-Lite><item>{inner}</item></DIDL-Lite>")
    }

    #[test]
    fn a_song_from_the_queue() {
        let position = Position {
            track: 3,
            metadata: didl(
                "<dc:title>Aline</dc:title><dc:creator>Christophe</dc:creator><upnp:album>Les Mots bleus</upnp:album>",
            ),
            ..Position::default()
        };
        let media = Media {
            uri: "x-rincon-queue:RINCON_A1#0".into(),
            metadata: String::new(),
        };
        let reading = Reading {
            name: "Salon".into(),
            transport: "PLAYING".into(),
            position,
            media,
            volume: 25,
            muted: false,
            group: vec!["media_player.cuisine".into()],
            ..Reading::default()
        };
        let (state, attributes) = entity(&reading);
        assert_eq!(state, "playing");
        assert_eq!(attributes["media_title"], "Aline");
        assert_eq!(attributes["media_artist"], "Christophe");
        assert_eq!(attributes["media_album_name"], "Les Mots bleus");
        assert_eq!(attributes["volume_level"], 0.25);
        assert_eq!(attributes["is_volume_muted"], false);
        assert_eq!(attributes["group_members"], json!(["media_player.cuisine"]));
        assert!(!attributes.contains_key("media_channel"));
    }

    #[test]
    fn a_radio_station() {
        let position = Position {
            metadata: didl(
                "<dc:title>x-sonosapi-stream:s15200?sid=254</dc:title><r:streamContent>Daft Punk - Get Lucky</r:streamContent>",
            ),
            ..Position::default()
        };
        let media = Media {
            uri: "x-sonosapi-stream:s15200?sid=254".into(),
            metadata: didl("<dc:title>FIP</dc:title>"),
        };
        let details = details(&position, &media);
        assert_eq!(details.title.as_deref(), Some("Get Lucky"));
        assert_eq!(details.artist.as_deref(), Some("Daft Punk"));
        assert_eq!(details.channel.as_deref(), Some("FIP"));
    }

    #[test]
    fn tv_line_in_and_idle() {
        let tv = Media {
            uri: "x-sonos-htastream:RINCON_A1:spdif".into(),
            ..Media::default()
        };
        assert_eq!(
            details(&Position::default(), &tv).title.as_deref(),
            Some("TV")
        );
        let line = Media {
            uri: "x-rincon-stream:RINCON_A1".into(),
            ..Media::default()
        };
        assert_eq!(
            details(&Position::default(), &line).title.as_deref(),
            Some("Entrée audio")
        );
        // À l'arrêt : pas de titre affiché.
        let reading = Reading {
            name: "Cuisine".into(),
            transport: "STOPPED".into(),
            position: Position {
                metadata: didl("<dc:title>Vieux titre</dc:title>"),
                ..Position::default()
            },
            ..Reading::default()
        };
        let (state, attributes) = entity(&reading);
        assert_eq!(state, "idle");
        assert!(!attributes.contains_key("media_title"));
        assert_eq!(state_name("PAUSED_PLAYBACK"), "paused");
        assert_eq!(state_name("TRANSITIONING"), "playing");
        assert_eq!(state_name("NO_MEDIA_PRESENT"), "idle");
    }

    fn playing_song(rel_time: &str) -> Reading {
        Reading {
            name: "Salon".into(),
            transport: "PLAYING".into(),
            position: Position {
                metadata: didl("<dc:title>Aline</dc:title>"),
                rel_time: rel_time.into(),
                duration: "0:03:45".into(),
                ..Position::default()
            },
            media: Media {
                uri: "x-rincon-queue:RINCON_A1#0".into(),
                metadata: String::new(),
            },
            play_mode: Some("SHUFFLE_NOREPEAT".into()),
            ..Reading::default()
        }
    }

    fn at(text: &str) -> jiff::Timestamp {
        text.parse().unwrap_or_else(|_| unreachable!())
    }

    #[test]
    fn duration_shuffle_and_repeat() {
        let (state, attributes) = entity(&playing_song("0:01:00"));
        assert_eq!(state, "playing");
        assert_eq!(attributes["media_duration"], 225);
        assert_eq!(attributes["shuffle"], true);
        assert_eq!(attributes["repeat"], "off");
        // Une radio : ni durée, ni aléatoire.
        let mut radio = playing_song("0:12:00");
        radio.position.duration = "NOT_IMPLEMENTED".into();
        radio.media.uri = "x-sonosapi-stream:s15200?sid=254".into();
        let (_, attributes) = entity(&radio);
        assert!(!attributes.contains_key("media_duration"));
        assert!(!attributes.contains_key("shuffle"));
    }

    #[test]
    fn the_position_changes_only_when_it_drifts() {
        let start = at("2026-10-22T20:00:00Z");
        let reading = playing_song("0:01:00");
        let (state, mut first) = entity(&reading);
        track_position(&mut first, state, &reading, None, start);
        assert_eq!(first["media_position"], 60);
        assert_eq!(first["media_position_updated_at"], "2026-10-22T20:00:00Z");
        // 3 s plus tard, le morceau a avancé de 3 s : rien ne change.
        let reading = playing_song("0:01:03");
        let (state, mut next) = entity(&reading);
        let later = at("2026-10-22T20:00:03.400Z");
        track_position(&mut next, state, &reading, Some(("playing", &first)), later);
        assert_eq!(next, first);
        // On a sauté plus loin dans le morceau : nouvelle position.
        let reading = playing_song("0:02:30");
        let (state, mut jumped) = entity(&reading);
        track_position(
            &mut jumped,
            state,
            &reading,
            Some(("playing", &first)),
            later,
        );
        assert_eq!(jumped["media_position"], 150);
        assert_eq!(jumped["media_position_updated_at"], "2026-10-22T20:00:03Z");
        // En pause, la position ne doit pas bouger.
        let mut paused = playing_song("0:01:00");
        paused.transport = "PAUSED_PLAYBACK".into();
        let (state, mut held) = entity(&paused);
        track_position(&mut held, state, &paused, None, start);
        let (state, mut still) = entity(&paused);
        track_position(&mut still, state, &paused, Some(("paused", &held)), later);
        assert_eq!(still, held);
    }

    #[test]
    fn times_and_modes() {
        assert_eq!(seconds("0:03:45"), Some(225));
        assert_eq!(seconds("1:00:00"), Some(3600));
        assert_eq!(seconds("NOT_IMPLEMENTED"), None);
        assert_eq!(seconds("0:99:00"), None);
        assert_eq!(seconds("3:45"), None);
        assert_eq!(seconds(""), None);
        assert_eq!(hms(225), "0:03:45");
        assert_eq!(hms(3725), "1:02:05");
        for mode in [
            "NORMAL",
            "REPEAT_ALL",
            "REPEAT_ONE",
            "SHUFFLE_NOREPEAT",
            "SHUFFLE",
            "SHUFFLE_REPEAT_ONE",
        ] {
            let (shuffle, repeat) = play_mode(mode).unwrap_or_else(|| unreachable!());
            assert_eq!(mode_name(shuffle, repeat), Some(mode));
        }
        assert_eq!(play_mode("PARTY"), None);
        assert_eq!(mode_name(true, "toujours"), None);
    }
}
