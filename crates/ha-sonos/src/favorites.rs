//! Les favoris Sonos (étape 91) : ce qu'on a mis en favori dans l'appli
//! Sonos (une playlist YouTube Music, une radio, un album…). Ils sont
//! communs à toute la maison et rangés dans chaque enceinte, dans le
//! dossier `FV:2` de son service `ContentDirectory`.
//!
//! Chaque favori a un nom, une adresse (`res`) et ses métadonnées
//! (`resMD`, du DIDL) : c'est tout ce qu'il faut pour le relancer.

use ha_core::Attributes;
use serde_json::json;

use crate::api::Media;
use crate::xml;

/// Au plus autant de favoris (l'appli Sonos en accepte bien moins en
/// pratique).
pub const MAX: usize = 100;
/// Un nom de favori trop long est coupé.
const MAX_TITLE: usize = 80;

/// Un favori qu'on sait lancer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Favorite {
    pub title: String,
    pub uri: String,
    pub metadata: String,
    /// Le service : « YouTube Music », « TuneIn »…
    pub service: String,
}

/// Comment le lancer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// Une radio ou une entrée : on la charge directement.
    Direct,
    /// Une playlist, un album, un morceau : on vide la file d'attente, on
    /// l'y ajoute, puis on lit la file.
    Queue,
}

impl Favorite {
    #[must_use]
    pub fn launch(&self) -> Launch {
        const DIRECT: [&str; 9] = [
            "x-sonosapi-stream:",
            "x-sonosapi-radio:",
            "x-sonosapi-hls:",
            "x-rincon-mp3radio:",
            "hls-radio:",
            "aac:",
            "x-rincon-stream:",
            "x-sonos-htastream:",
            "x-sonos-vli:",
        ];
        if DIRECT.iter().any(|prefix| self.uri.starts_with(prefix)) {
            Launch::Direct
        } else {
            Launch::Queue
        }
    }
}

/// Les favoris d'après la réponse à `Browse` (le DIDL déjà décodé une
/// fois). Ceux sans adresse (les raccourcis « Sonos Radio » de l'appli) ne
/// se lancent pas d'ici : ils sont laissés de côté. Un nom en double
/// n'est gardé qu'une fois.
#[must_use]
pub fn parse(didl: &str) -> Vec<Favorite> {
    let mut list: Vec<Favorite> = Vec::new();
    for item in xml::elements(didl, "item", MAX) {
        let field = |name| {
            xml::element_text(item, name)
                .map(|text| clean(&text))
                .unwrap_or_default()
        };
        let title: String = field("title").chars().take(MAX_TITLE).collect();
        let uri = xml::element_text(item, "res")
            .unwrap_or_default()
            .trim()
            .to_owned();
        if title.is_empty() || uri.is_empty() || list.iter().any(|f| f.title == title) {
            continue;
        }
        list.push(Favorite {
            title,
            uri,
            metadata: xml::element_text(item, "resMD").unwrap_or_default(),
            service: field("description"),
        });
    }
    list
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Ajoute `source_list` (les noms des favoris) aux attributs de
/// l'enceinte, comme Home Assistant, et `source` quand ce qui est chargé
/// est l'un d'eux (une radio : une playlist passe par la file d'attente et
/// ne se reconnaît pas).
pub fn describe(favorites: &[Favorite], media: &Media, attributes: &mut Attributes) {
    if favorites.is_empty() {
        return;
    }
    let names: Vec<&str> = favorites.iter().map(|f| f.title.as_str()).collect();
    attributes.insert("source_list".into(), json!(names));
    if let Some(current) = favorites
        .iter()
        .find(|f| f.launch() == Launch::Direct && f.uri == media.uri)
    {
        attributes.insert("source".into(), json!(current.title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Comme une vraie enceinte : chaque favori, avec ses métadonnées
    /// échappées dans `r:resMD`.
    fn item(title: &str, uri: &str, service: &str) -> String {
        let meta = xml::escape(&format!(
            "<DIDL-Lite><item id=\"x\"><dc:title>{}</dc:title><upnp:class>object.container.playlistContainer</upnp:class><desc id=\"cdudn\">SA_RINCON_X</desc></item></DIDL-Lite>",
            xml::escape(title)
        ));
        format!(
            "<item id=\"FV:2/1\" parentID=\"FV:2\"><dc:title>{}</dc:title><upnp:class>object.itemobject.item.sonos-favorite</upnp:class><res protocolInfo=\"x\">{}</res><r:type>instantPlay</r:type><r:description>{service}</r:description><r:resMD>{meta}</r:resMD></item>",
            xml::escape(title),
            xml::escape(uri)
        )
    }

    #[test]
    fn favorites_from_the_speaker() {
        let didl = format!(
            "<DIDL-Lite>{}{}{}{}</DIDL-Lite>",
            item(
                "Rock & Pop",
                "x-rincon-cpcontainer:1006206cplaylist?sid=284&flags=8300&sn=7",
                "YouTube Music"
            ),
            item("FIP", "x-sonosapi-stream:s15200?sid=254", "TuneIn"),
            // Un raccourci de l'appli, sans adresse : laissé de côté.
            "<item><dc:title>Découvrez Sonos Radio</dc:title><res></res><r:type>shortcut</r:type></item>",
            // Un nom en double : gardé une fois.
            item("FIP", "x-sonosapi-stream:autre", "TuneIn"),
        );
        let list = parse(&didl);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "Rock & Pop");
        assert_eq!(
            list[0].uri,
            "x-rincon-cpcontainer:1006206cplaylist?sid=284&flags=8300&sn=7"
        );
        assert_eq!(list[0].service, "YouTube Music");
        assert!(list[0].metadata.starts_with("<DIDL-Lite>"));
        assert_eq!(
            xml::element_text(&list[0].metadata, "title").as_deref(),
            Some("Rock & Pop")
        );
        assert_eq!(list[0].launch(), Launch::Queue);
        assert_eq!(list[1].launch(), Launch::Direct);
        assert_eq!(parse("pas du xml"), []);
    }

    #[test]
    fn attributes_of_the_speaker() {
        let favorites = parse(&format!(
            "<DIDL-Lite>{}{}</DIDL-Lite>",
            item("Ma playlist", "x-rincon-cpcontainer:abc", "YouTube Music"),
            item("FIP", "x-sonosapi-stream:s15200?sid=254", "TuneIn"),
        ));
        let mut attributes = Attributes::new();
        let radio = Media {
            uri: "x-sonosapi-stream:s15200?sid=254".into(),
            metadata: String::new(),
        };
        describe(&favorites, &radio, &mut attributes);
        assert_eq!(attributes["source_list"], json!(["Ma playlist", "FIP"]));
        assert_eq!(attributes["source"], json!("FIP"));
        let mut attributes = Attributes::new();
        let queue = Media {
            uri: "x-rincon-queue:RINCON_A#0".into(),
            metadata: String::new(),
        };
        describe(&favorites, &queue, &mut attributes);
        assert!(!attributes.contains_key("source"));
        let mut attributes = Attributes::new();
        describe(&[], &queue, &mut attributes);
        assert!(attributes.is_empty());
    }
}
