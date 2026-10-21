//! Une enceinte comme entité `media_player`, comme dans Home Assistant :
//! son état (`playing`, `paused`, `idle`) et ce qu'elle joue.

use ha_core::Attributes;
use serde_json::json;

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
}

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
    if !reading.group.is_empty() {
        attributes.insert("group_members".into(), json!(reading.group));
    }
    (state, attributes)
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
}
