//! Le cerveau (docs/cerveau.md) : quand Assist ne comprend pas une phrase,
//! le hub peut la passer au service « cerveau », où Claude Code y répond avec
//! l'abonnement Claude de la maison. Optionnel : sans `HA_CERVEAU`, rien ne
//! change et rien ne sort de la maison.
//!
//! Seulement pour les comptes de la famille et l'administrateur (jamais un
//! invité), pour une phrase d'au moins deux mots qu'Assist n'a pas su lire
//! (pas quand il demande « Lequel ? »), et au plus [`MAX_PER_HOUR`] phrases
//! par heure : le bruit pris pour une phrase après « Maison » n'use pas
//! l'abonnement. La phrase n'est jamais écrite dans les logs.
//!
//! Le cerveau est sur le réseau interne de Docker : `POST /demande` en HTTP
//! simple, avec la clé partagée (`HA_CERVEAU_CLE_FILE`). Un petit client
//! écrit ici suffit (une requête, `Connection: close`).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ha_assist::NotUnderstood;
use ha_auth::Role;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

/// Le temps laissé au cerveau (lui-même coupe Claude à 30 s).
const TIMEOUT: Duration = Duration::from_secs(40);
/// Au-delà, les phrases suivantes reçoivent la réponse d'Assist seule.
pub(crate) const MAX_PER_HOUR: usize = 40;
const HOUR: Duration = Duration::from_hours(1);
/// La plus longue phrase passée (comme Assist et le cerveau).
const MAX_TEXT: usize = ha_assist::MAX_TEXT_CHARS;
/// La plus longue réponse lue (le cerveau coupe déjà à 600).
const MAX_REPLY: usize = 600;
/// La réponse HTTP la plus grande acceptée.
const MAX_RESPONSE: u64 = 16 * 1024;
/// Une clé plus courte se devinerait trop facilement.
const MIN_KEY_LEN: usize = 32;

/// L'adresse du cerveau, sa clé et sa limite de débit.
pub(crate) struct Cerveau {
    host: String,
    port: u16,
    key: Zeroizing<String>,
    asked: Mutex<VecDeque<Instant>>,
}

impl std::fmt::Debug for Cerveau {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cerveau")
            .field("host", &self.host)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct Reply {
    reponse: String,
}

impl Cerveau {
    /// `address` : `http://cerveau:8790` ; `key` : la clé partagée.
    pub(crate) fn new(address: &str, key: &str) -> Result<Self, String> {
        let url = url::Url::parse(address.trim())
            .map_err(|error| format!("adresse du cerveau illisible ({error})"))?;
        if url.scheme() != "http" {
            return Err("adresse du cerveau : http://hôte:port attendu".into());
        }
        let host = url
            .host_str()
            .ok_or("adresse du cerveau : hôte manquant")?
            .to_owned();
        let port = url.port_or_known_default().unwrap_or(80);
        let key = key.trim();
        if key.chars().count() < MIN_KEY_LEN {
            return Err(format!(
                "clé du cerveau trop courte : {MIN_KEY_LEN} caractères au moins"
            ));
        }
        if !key.chars().all(|c| c.is_ascii_graphic()) {
            return Err("clé du cerveau : lettres, chiffres et signes seulement".into());
        }
        Ok(Self {
            host,
            port,
            key: Zeroizing::new(key.to_owned()),
            asked: Mutex::default(),
        })
    }

    /// Compte une phrase ; `false` si la limite de l'heure est atteinte.
    fn take(&self, now: Instant) -> bool {
        let mut asked = self
            .asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while asked
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= HOUR)
        {
            asked.pop_front();
        }
        if asked.len() >= MAX_PER_HOUR {
            return false;
        }
        asked.push_back(now);
        true
    }

    /// Pose la phrase au cerveau ; `None` s'il ne répond pas (Assist donne
    /// alors sa propre réponse). `username` : pour les logs seulement.
    pub(crate) async fn ask(&self, username: &str, text: &str) -> Option<String> {
        if !self.take(Instant::now()) {
            tracing::warn!(
                "cerveau : limite de {MAX_PER_HOUR} phrases par heure atteinte ({username})"
            );
            return None;
        }
        let start = Instant::now();
        match tokio::time::timeout(TIMEOUT, self.request(text)).await {
            Ok(Ok(reply)) => {
                tracing::info!(
                    "cerveau : réponse pour {username} en {} ms",
                    start.elapsed().as_millis()
                );
                Some(reply)
            }
            Ok(Err(error)) => {
                tracing::warn!("cerveau : pas de réponse pour {username} ({error})");
                None
            }
            Err(_) => {
                tracing::warn!("cerveau : pas de réponse pour {username} (délai dépassé)");
                None
            }
        }
    }

    async fn request(&self, text: &str) -> Result<String, String> {
        let body = serde_json::json!({ "texte": text }).to_string();
        let head = Zeroizing::new(format!(
            "POST /demande HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            self.host,
            self.port,
            self.key.as_str(),
            body.len()
        ));
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|error| format!("connexion impossible : {}", error.kind()))?;
        stream
            .write_all(head.as_bytes())
            .await
            .and(stream.write_all(body.as_bytes()).await)
            .map_err(|error| format!("envoi impossible : {}", error.kind()))?;
        let mut raw = Vec::new();
        (&mut stream)
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut raw)
            .await
            .map_err(|error| format!("lecture impossible : {}", error.kind()))?;
        if raw.len() as u64 > MAX_RESPONSE {
            return Err("réponse trop grande".into());
        }
        parse(&raw)
    }
}

/// La réponse HTTP du cerveau : `200` et `{"reponse": "…"}`.
fn parse(raw: &[u8]) -> Result<String, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("réponse HTTP incomplète")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "en-têtes illisibles")?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("statut HTTP manquant")?;
    if status != "200" {
        return Err(format!("statut {status}"));
    }
    let reply: Reply =
        serde_json::from_slice(&raw[split + 4..]).map_err(|_| "réponse illisible")?;
    let reply = clean(&reply.reponse);
    if reply.is_empty() {
        return Err("réponse vide".into());
    }
    Ok(reply)
}

/// Une seule ligne, sans caractère de contrôle, pas trop longue.
fn clean(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words: Vec<&str> = flat.split_whitespace().collect();
    let joined = words.join(" ");
    if joined.chars().count() <= MAX_REPLY {
        return joined;
    }
    let cut: String = joined.chars().take(MAX_REPLY).collect();
    format!("{}…", cut.trim_end())
}

/// La phrase peut-elle aller au cerveau ? Pas pour un invité, pas pour une
/// phrase d'un mot ou trop longue, et pas quand Assist sait quoi demander
/// (« Lequel ? », « Que faut-il ajouter ? »).
pub(crate) fn eligible(role: Role, text: &str, why: &NotUnderstood) -> bool {
    let reason = matches!(
        why,
        NotUnderstood::NoAction | NotUnderstood::NoDevice | NotUnderstood::Unsupported(_)
    );
    reason
        && role != Role::Guest
        && text.split_whitespace().count() >= 2
        && text.chars().count() <= MAX_TEXT
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    const KEY: &str = "une-cle-de-cerveau-assez-longue-pour-essai";

    /// Un faux cerveau : lit une requête et renvoie `response` ; rend ce
    /// qu'il a reçu.
    async fn fake(response: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                seen.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&seen);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length: usize = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if seen.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(seen).unwrap()
        });
        (address, task)
    }

    #[test]
    fn address_and_key_are_checked() {
        assert!(Cerveau::new("http://cerveau:8790", KEY).is_ok());
        assert!(Cerveau::new("https://cerveau:8790", KEY).is_err());
        assert!(Cerveau::new("cerveau", KEY).is_err());
        assert!(Cerveau::new("http://cerveau:8790", "courte").is_err());
        assert!(Cerveau::new("http://cerveau:8790", &format!("{KEY} x")).is_err());
        // La clé n'apparaît jamais dans les logs.
        let cerveau = Cerveau::new("http://cerveau:8790", KEY).unwrap();
        assert!(!format!("{cerveau:?}").contains(KEY));
    }

    #[test]
    fn only_family_and_real_misunderstandings() {
        let text = "est-ce que je dois arroser";
        assert!(eligible(Role::Family, text, &NotUnderstood::NoAction));
        assert!(eligible(Role::Admin, text, &NotUnderstood::NoDevice));
        assert!(!eligible(Role::Guest, text, &NotUnderstood::NoAction));
        assert!(!eligible(Role::Family, "euh", &NotUnderstood::NoAction));
        assert!(!eligible(
            Role::Family,
            text,
            &NotUnderstood::Ambiguous(vec!["a".into(), "b".into()])
        ));
        assert!(!eligible(Role::Family, text, &NotUnderstood::NothingToAdd));
        let long = "mot ".repeat(MAX_TEXT);
        assert!(!eligible(Role::Family, &long, &NotUnderstood::NoAction));
    }

    #[test]
    fn at_most_forty_an_hour() {
        let cerveau = Cerveau::new("http://cerveau:8790", KEY).unwrap();
        let start = Instant::now();
        for _ in 0..MAX_PER_HOUR {
            assert!(cerveau.take(start));
        }
        assert!(!cerveau.take(start + Duration::from_mins(59)));
        assert!(cerveau.take(start + HOUR));
    }

    #[test]
    fn replies_are_cleaned() {
        assert_eq!(clean("  Il fait\n21 degrés.\t"), "Il fait 21 degrés.");
        let long = clean(&"a ".repeat(MAX_REPLY));
        assert!(long.chars().count() <= MAX_REPLY + 1 && long.ends_with('…'));
        assert_eq!(
            parse(b"HTTP/1.1 502 Bad Gateway\r\n\r\n{}"),
            Err("statut 502".into())
        );
        assert!(parse(b"HTTP/1.1 200 OK\r\n\r\n{\"reponse\":\"  \"}").is_err());
        assert!(parse(b"HTTP/1.1 200 OK\r\n\r\npas du json").is_err());
    }

    #[tokio::test]
    async fn asks_with_the_key_and_reads_the_reply() {
        let (address, seen) = fake(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n\
             {\"reponse\":\"Il fait 21 degrés au salon.\"}",
        )
        .await;
        let cerveau = Cerveau::new(&address, KEY).unwrap();
        let reply = cerveau.ask("david", "quel temps fait-il dedans").await;
        assert_eq!(reply.as_deref(), Some("Il fait 21 degrés au salon."));
        let seen = seen.await.unwrap();
        assert!(seen.starts_with("POST /demande HTTP/1.1\r\n"));
        assert!(seen.contains(&format!("Authorization: Bearer {KEY}\r\n")));
        assert!(seen.ends_with("{\"texte\":\"quel temps fait-il dedans\"}"));
    }

    #[tokio::test]
    async fn no_reply_when_the_cerveau_fails() {
        let (address, _seen) = fake("HTTP/1.1 503 Service Unavailable\r\n\r\n{}").await;
        let cerveau = Cerveau::new(&address, KEY).unwrap();
        assert_eq!(cerveau.ask("david", "fais un truc").await, None);
        // Personne n'écoute : pas de réponse non plus.
        let closed = Cerveau::new("http://127.0.0.1:9", KEY).unwrap();
        assert_eq!(closed.ask("david", "fais un truc").await, None);
    }
}
