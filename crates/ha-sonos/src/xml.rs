//! Un tout petit lecteur XML, juste ce qu'il faut pour les réponses des
//! Sonos : le texte d'un élément, et les attributs d'une suite de balises.
//!
//! Les réponses UPnP sont simples et toujours de la même forme : pas besoin
//! d'une bibliothèque complète (et d'une dépendance de plus). Tout ce qui
//! n'a pas la forme attendue donne `None`, jamais une panique.

/// Le texte du premier élément nommé `name` (sans son préfixe : `title`
/// trouve `<dc:title>`), entités décodées.
#[must_use]
pub fn element_text(xml: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = xml[from..].find('<') {
        let start = from + found;
        let tag = Tag::parse(&xml[start..])?;
        from = start + tag.length;
        if tag.closing || local(tag.name) != name {
            continue;
        }
        if tag.self_closing {
            return Some(String::new());
        }
        let content = &xml[from..];
        let end = find_closing(content, tag.name)?;
        return Some(unescape(&content[..end]));
    }
    None
}

/// Le contenu **brut** (entités non décodées) de chaque élément nommé
/// `name`, dans l'ordre : de quoi relire ensuite chacun avec
/// [`element_text`]. Les éléments du même nom imbriqués restent dans leur
/// parent. Au plus `max` éléments.
#[must_use]
pub fn elements<'a>(xml: &'a str, name: &str, max: usize) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut from = 0;
    while found.len() < max
        && let Some(offset) = xml[from..].find('<')
    {
        let start = from + offset;
        let Some(tag) = Tag::parse(&xml[start..]) else {
            break;
        };
        from = start + tag.length;
        if tag.closing || local(tag.name) != name {
            continue;
        }
        if tag.self_closing {
            found.push("");
            continue;
        }
        let content = &xml[from..];
        let Some(end) = find_closing(content, tag.name) else {
            break;
        };
        found.push(&content[..end]);
        from += end;
    }
    found
}

/// Les balises ouvrantes, dans l'ordre : leur nom (sans préfixe) et leurs
/// attributs décodés.
#[must_use]
pub fn start_tags(xml: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut tags = Vec::new();
    let mut from = 0;
    while let Some(found) = xml[from..].find('<') {
        let start = from + found;
        let Some(tag) = Tag::parse(&xml[start..]) else {
            break;
        };
        from = start + tag.length;
        if !tag.closing && !tag.name.is_empty() {
            tags.push((local(tag.name).to_owned(), attributes(tag.attributes)));
        }
    }
    tags
}

/// La valeur d'un attribut parmi ceux rendus par [`start_tags`].
#[must_use]
pub fn attribute<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

struct Tag<'a> {
    name: &'a str,
    attributes: &'a str,
    closing: bool,
    self_closing: bool,
    /// Longueur de la balise, `<` et `>` compris.
    length: usize,
}

impl<'a> Tag<'a> {
    /// `text` commence par `<`.
    fn parse(text: &'a str) -> Option<Self> {
        // Déclaration, commentaire, CDATA… : on saute jusqu'au `>`.
        let end = find_tag_end(text)?;
        let inner = &text[1..end];
        if inner.starts_with('?') || inner.starts_with('!') {
            return Some(Self {
                name: "",
                attributes: "",
                closing: true,
                self_closing: true,
                length: end + 1,
            });
        }
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, inner),
        };
        let (self_closing, inner) = match inner.strip_suffix('/') {
            Some(rest) => (true, rest),
            None => (false, inner),
        };
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        Some(Self {
            name: &inner[..name_end],
            attributes: &inner[name_end..],
            closing,
            self_closing,
            length: end + 1,
        })
    }
}

/// La position du `>` qui ferme la balise, en sautant ceux qui sont dans
/// une valeur d'attribut entre guillemets.
fn find_tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (index, c) in text.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

/// Où commence `</name>` (même nom, préfixe compris), en tenant compte des
/// éléments du même nom imbriqués.
fn find_closing(content: &str, name: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut from = 0;
    while let Some(found) = content[from..].find('<') {
        let start = from + found;
        let tag = Tag::parse(&content[start..])?;
        from = start + tag.length;
        if tag.name != name {
            continue;
        }
        if tag.closing {
            if depth == 0 {
                return Some(start);
            }
            depth -= 1;
        } else if !tag.self_closing {
            depth += 1;
        }
    }
    None
}

fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn attributes(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(equal) = rest.find('=') {
        let key = rest[..equal].trim();
        let after = rest[equal + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let Some(close) = after[1..].find(quote) else {
            break;
        };
        out.push((local(key).to_owned(), unescape(&after[1..=close])));
        rest = &after[close + 2..];
    }
    out
}

/// Décode `&lt;`, `&gt;`, `&amp;`, `&quot;`, `&apos;` et `&#…;`. Une entité
/// inconnue est laissée telle quelle.
#[must_use]
pub fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        let decoded = after.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &after[1..end];
            let c = match entity {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .map_or_else(
                        || entity.strip_prefix('#').and_then(|n| n.parse::<u32>().ok()),
                        |hex| u32::from_str_radix(hex, 16).ok(),
                    )
                    .and_then(char::from_u32)
                    .filter(|c| *c != '\0'),
            };
            c.map(|c| (c, end))
        });
        if let Some((c, end)) = decoded {
            out.push(c);
            rest = &after[end + 1..];
        } else {
            out.push('&');
            rest = &after[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Le contraire : pour mettre un texte dans du XML.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_text_from_a_soap_answer() {
        let xml = r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><u:GetVolumeResponse xmlns:u="urn:schemas-upnp-org:service:RenderingControl:1"><CurrentVolume>12</CurrentVolume></u:GetVolumeResponse></s:Body></s:Envelope>"#;
        assert_eq!(element_text(xml, "CurrentVolume").as_deref(), Some("12"));
        assert_eq!(element_text(xml, "Absent"), None);
        // Préfixe ignoré, entités décodées, élément vide.
        let didl = "<DIDL-Lite><item><dc:title>Tom &amp; Jerry &#233;t&#xE9; &lt;3</dc:title><upnp:album/></item></DIDL-Lite>";
        assert_eq!(
            element_text(didl, "title").as_deref(),
            Some("Tom & Jerry été <3")
        );
        assert_eq!(element_text(didl, "album").as_deref(), Some(""));
        // Un texte qui contient du XML échappé (les métadonnées DIDL).
        let nested = "<TrackMetaData>&lt;DIDL-Lite&gt;&lt;dc:title&gt;Titre&lt;/dc:title&gt;&lt;/DIDL-Lite&gt;</TrackMetaData>";
        let inner = element_text(nested, "TrackMetaData").unwrap_or_default();
        assert_eq!(element_text(&inner, "title").as_deref(), Some("Titre"));
    }

    #[test]
    fn nested_elements_of_the_same_name() {
        let xml = "<a><b>1<b>2</b>3</b></a>";
        assert_eq!(element_text(xml, "b").as_deref(), Some("1<b>2</b>3"));
    }

    #[test]
    fn several_elements() {
        let xml = "<DIDL-Lite><item id=\"1\"><dc:title>A &amp; B</dc:title></item><container/><item><dc:title>C</dc:title><item>dedans</item></item><item/></DIDL-Lite>";
        let items = elements(xml, "item", 10);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], "<dc:title>A &amp; B</dc:title>");
        assert_eq!(element_text(items[0], "title").as_deref(), Some("A & B"));
        assert_eq!(element_text(items[1], "title").as_deref(), Some("C"));
        assert_eq!(items[2], "");
        assert_eq!(elements(xml, "item", 1).len(), 1);
        assert_eq!(elements("<item>pas fermé", "item", 10), [] as [&str; 0]);
    }

    #[test]
    fn tags_and_attributes() {
        let xml = r#"<ZoneGroups><ZoneGroup Coordinator="RINCON_A" ID="x"><ZoneGroupMember UUID="RINCON_A" ZoneName="Salon &amp; cuisine" Location='http://192.168.1.30:1400/xml/device_description.xml'><Satellite UUID="RINCON_S" Invisible="1"/></ZoneGroupMember></ZoneGroup></ZoneGroups>"#;
        let tags = start_tags(xml);
        let names: Vec<&str> = tags.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["ZoneGroups", "ZoneGroup", "ZoneGroupMember", "Satellite"]
        );
        assert_eq!(attribute(&tags[1].1, "Coordinator"), Some("RINCON_A"));
        assert_eq!(attribute(&tags[2].1, "ZoneName"), Some("Salon & cuisine"));
        assert_eq!(
            attribute(&tags[2].1, "Location"),
            Some("http://192.168.1.30:1400/xml/device_description.xml")
        );
        assert_eq!(attribute(&tags[3].1, "Invisible"), Some("1"));
    }

    #[test]
    fn broken_xml_gives_nothing() {
        for xml in ["<a", "<a>texte", "<a b=\"1>", "&", ""] {
            assert_eq!(element_text(xml, "a"), None, "{xml}");
            let _ = start_tags(xml);
        }
        assert_eq!(unescape("a &unknown; &#0; &"), "a &unknown; &#0; &");
    }

    #[test]
    fn escape_round_trip() {
        let text = r#"<"Tom" & 'Jerry'>"#;
        assert_eq!(unescape(&escape(text)), text);
    }
}
