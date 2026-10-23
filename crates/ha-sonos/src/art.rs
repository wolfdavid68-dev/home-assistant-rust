//! Les **pochettes** (étape 92) : l'image de l'album qui joue, et celles
//! des favoris.
//!
//! Les métadonnées des enceintes donnent l'adresse de la pochette
//! (`albumArtURI`) : souvent `/getaa?…` sur l'enceinte elle-même (qui va la
//! chercher chez le service de musique), parfois une adresse `https://`
//! chez le service. Le tableau de bord n'affiche que des images de notre
//! serveur : on la télécharge ici, on la range dans
//! [`ha_core::covers::Covers`], et l'entité la montre dans
//! `entity_picture`.
//!
//! Sécurité : on ne télécharge que depuis une enceinte connue (en HTTP sur
//! son port 1400) ou une adresse HTTPS sur Internet, jamais ailleurs sur
//! le réseau local. La réponse doit être un vrai JPEG ou PNG, et pas trop
//! grosse.

use std::collections::HashMap;
use std::time::Duration;

use ha_core::covers::{KEY_LENGTH, MAX_COVER};
use sha2::{Digest, Sha256};
use url::{Host, Url};

use crate::xml;

/// Une pochette qui ne vient pas en 10 s est abandonnée.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Une pochette qui a échoué n'est retentée qu'après ce délai.
pub const RETRY_AFTER: Duration = Duration::from_mins(10);

/// L'adresse de la pochette dans des métadonnées DIDL, telle quelle.
#[must_use]
pub fn in_metadata(didl: &str) -> Option<String> {
    xml::element_text(didl, "albumArtURI")
        .map(|art| art.trim().to_owned())
        .filter(|art| !art.is_empty())
}

/// L'adresse complète à télécharger, ou `None` si elle n'est pas permise.
///
/// - `/getaa?…` : sur l'enceinte `speaker` (`192.168.1.30:1400`) ;
/// - `http://…` : seulement vers une des enceintes `speakers` ;
/// - `https://…` : un nom de domaine sur Internet (pas une adresse IP ni
///   `localhost`).
#[must_use]
pub fn resolve(art: &str, speaker: &str, speakers: &[String]) -> Option<String> {
    if art.len() > 2048 {
        return None;
    }
    if art.starts_with('/') && !art.starts_with("//") {
        return Url::parse(&format!("http://{speaker}{art}"))
            .ok()
            .filter(|url| authority(url).is_some_and(|a| a == speaker))
            .map(String::from);
    }
    let url = Url::parse(art).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    match url.scheme() {
        "http" => {
            let authority = authority(&url)?;
            speakers.contains(&authority).then(|| url.into())
        }
        "https" => match url.host()? {
            // `Url` met déjà le nom en minuscules.
            Host::Domain(name)
                if name.contains('.') && name.rsplit('.').next() != Some("local") =>
            {
                Some(url.into())
            }
            _ => None,
        },
        _ => None,
    }
}

/// `hôte:port` d'une adresse HTTP (le port 80 compris).
fn authority(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

/// La clé de la pochette : une empreinte de son adresse.
#[must_use]
pub fn key(url: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    Sha256::digest(url.as_bytes())
        .iter()
        .take(KEY_LENGTH / 2)
        .flat_map(|byte| [byte >> 4, byte & 0xF])
        .map(|digit| char::from(HEX[usize::from(digit)]))
        .collect()
}

/// L'adresse où le tableau de bord la trouve.
#[must_use]
pub fn picture(key: &str) -> String {
    format!("/api/media_cover/{key}")
}

/// Télécharge une pochette (adresse déjà passée par [`resolve`]).
pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut response = http
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .header("Accept", "image/jpeg, image/png")
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("réponse {}", response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_COVER as u64)
    {
        return Err("pochette trop grosse".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| error.without_url().to_string())?
    {
        if bytes.len() + chunk.len() > MAX_COVER {
            return Err("pochette trop grosse".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Les pochettes à aller chercher, et celles qui ont échoué récemment.
#[derive(Debug, Default)]
pub struct Wanted {
    /// En cours de téléchargement.
    pub pending: std::collections::HashSet<String>,
    /// Clé → moment de l'échec.
    pub failed: HashMap<String, tokio::time::Instant>,
}

impl Wanted {
    /// Faut-il lancer le téléchargement de cette clé ? Si oui, elle est
    /// marquée « en cours ».
    pub fn start(&mut self, key: &str, now: tokio::time::Instant) -> bool {
        if self.pending.contains(key) {
            return false;
        }
        if let Some(at) = self.failed.get(key) {
            if now.duration_since(*at) < RETRY_AFTER {
                return false;
            }
            self.failed.remove(key);
        }
        self.pending.insert(key.to_owned());
        true
    }

    pub fn finish(&mut self, key: &str, ok: bool, now: tokio::time::Instant) {
        self.pending.remove(key);
        if !ok {
            // On oublie les échecs anciens pour ne pas grossir sans fin.
            self.failed
                .retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
            self.failed.insert(key.to_owned(), now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speakers() -> Vec<String> {
        vec!["192.168.1.30:1400".into(), "192.168.1.31:1400".into()]
    }

    #[test]
    fn only_speakers_and_internet_images() {
        let speakers = speakers();
        let resolve = |art| resolve(art, "192.168.1.30:1400", &speakers);
        assert_eq!(
            resolve("/getaa?s=1&u=x-sonos-http%3aabc").as_deref(),
            Some("http://192.168.1.30:1400/getaa?s=1&u=x-sonos-http%3aabc")
        );
        assert_eq!(
            resolve("http://192.168.1.31:1400/getaa?s=1").as_deref(),
            Some("http://192.168.1.31:1400/getaa?s=1")
        );
        assert_eq!(
            resolve("https://yt3.googleusercontent.com/abc=w544-h544").as_deref(),
            Some("https://yt3.googleusercontent.com/abc=w544-h544")
        );
        // Ailleurs sur le réseau local, ou avec des identifiants : non.
        assert_eq!(resolve("http://192.168.1.1/admin"), None);
        assert_eq!(resolve("http://192.168.1.30/getaa"), None);
        assert_eq!(resolve("https://192.168.1.1/x.jpg"), None);
        assert_eq!(resolve("https://localhost/x.jpg"), None);
        assert_eq!(resolve("https://box.local/x.jpg"), None);
        assert_eq!(resolve("https://a:b@images.example.com/x.jpg"), None);
        assert_eq!(resolve("//images.example.com/x.jpg"), None);
        // Une adresse relative reste sur l'enceinte.
        assert_eq!(
            resolve("/@192.168.1.1/x").as_deref(),
            Some("http://192.168.1.30:1400/@192.168.1.1/x")
        );
        assert_eq!(resolve("file:///etc/passwd"), None);
        assert_eq!(resolve(""), None);
    }

    #[test]
    fn keys_are_stable_and_valid() {
        let a = key("https://images.example.com/a.jpg");
        assert_eq!(a, key("https://images.example.com/a.jpg"));
        assert_ne!(a, key("https://images.example.com/b.jpg"));
        assert!(ha_core::covers::is_key(&a));
        assert_eq!(picture(&a), format!("/api/media_cover/{a}"));
    }

    #[test]
    fn art_from_metadata() {
        let didl = "<DIDL-Lite><item><upnp:albumArtURI>/getaa?s=1&amp;u=x</upnp:albumArtURI></item></DIDL-Lite>";
        assert_eq!(in_metadata(didl).as_deref(), Some("/getaa?s=1&u=x"));
        assert_eq!(in_metadata("<DIDL-Lite></DIDL-Lite>"), None);
    }

    #[test]
    fn failures_wait_before_a_retry() {
        let now = tokio::time::Instant::now();
        let mut wanted = Wanted::default();
        assert!(wanted.start("a", now));
        assert!(!wanted.start("a", now));
        wanted.finish("a", false, now);
        assert!(!wanted.start("a", now + Duration::from_secs(60)));
        assert!(wanted.start("a", now + RETRY_AFTER));
        wanted.finish("a", true, now + RETRY_AFTER);
        assert!(wanted.start("a", now + RETRY_AFTER));
    }
}
