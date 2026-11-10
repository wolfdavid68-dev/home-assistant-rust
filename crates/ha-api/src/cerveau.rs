//! Le cerveau (docs/cerveau.md) : quand Assist ne comprend pas une phrase,
//! le hub peut la passer au service « cerveau », où Claude Code y répond avec
//! l'abonnement Claude de la maison. Optionnel : sans `HA_CERVEAU`, rien ne
//! change et rien ne sort de la maison.
//!
//! Seulement pour les comptes de la famille et l'administrateur (jamais un
//! invité), pour une phrase d'au moins deux mots qu'Assist n'a pas su lire
//! (pas quand il demande « Lequel ? »), et au plus [`MAX_PER_HOUR`] demandes
//! par heure et `HA_CERVEAU_PAR_JOUR` par jour : le bruit pris pour une
//! phrase après « Maison » n'use pas l'abonnement. La phrase n'est jamais
//! écrite dans les logs.
//!
//! Le hub se souvient des derniers échanges de chacun pendant [`TALK`]
//! (pour « et pareil dans la chambre ») ; « merci » les oublie. Il joint
//! aussi les préférences de la famille et, pour « Pourquoi ? » et le résumé
//! du matin, des informations de la maison.
//!
//! Le cerveau est sur le réseau interne de Docker : `POST /demande` en HTTP
//! simple, avec la clé partagée (`HA_CERVEAU_CLE_FILE`). Un petit client
//! écrit ici suffit (une requête, `Connection: close`).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ha_assist::NotUnderstood;
use ha_auth::Role;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

/// Le temps laissé au cerveau (lui-même coupe Claude à 30 s).
const TIMEOUT: Duration = Duration::from_secs(40);
/// Au-delà, les phrases suivantes reçoivent la réponse d'Assist seule.
pub(crate) const MAX_PER_HOUR: usize = 40;
/// Par jour (24 heures glissantes), sans `HA_CERVEAU_PAR_JOUR`.
pub const DEFAULT_PER_DAY: usize = 150;
/// La plus grande limite par jour acceptée.
pub const MAX_PER_DAY: usize = 1000;
const HOUR: Duration = Duration::from_hours(1);
const DAY: Duration = Duration::from_hours(24);
/// Une conversation avec le cerveau s'oublie après ce temps sans phrase.
pub(crate) const TALK: Duration = Duration::from_mins(5);
/// Les échanges d'avant joints au plus (comme le cerveau).
pub(crate) const MAX_AVANT: usize = 3;
/// Les préférences de la famille : caractères au plus (comme le cerveau).
pub(crate) const MAX_PREFERENCES: usize = 2000;
/// Les informations de la maison jointes : caractères au plus.
pub(crate) const MAX_CONTEXTE: usize = 8000;
/// La plus longue phrase passée (comme Assist et le cerveau).
const MAX_TEXT: usize = ha_assist::MAX_TEXT_CHARS;
/// La plus longue réponse lue (le cerveau coupe déjà à 600).
const MAX_REPLY: usize = 600;
/// La réponse HTTP la plus grande acceptée.
const MAX_RESPONSE: u64 = 16 * 1024;
/// Une clé plus courte se devinerait trop facilement.
const MIN_KEY_LEN: usize = 32;

/// L'adresse du cerveau, sa clé, ses limites et les conversations en cours.
pub(crate) struct Cerveau {
    host: String,
    port: u16,
    key: Zeroizing<String>,
    per_day: usize,
    /// Les demandes des dernières 24 heures.
    asked: Mutex<VecDeque<Instant>>,
    /// Par compte : les derniers échanges.
    talks: Mutex<HashMap<i64, Talk>>,
}

impl std::fmt::Debug for Cerveau {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cerveau")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("per_day", &self.per_day)
            .finish_non_exhaustive()
    }
}

/// Les derniers échanges d'une personne avec le cerveau.
struct Talk {
    last: Instant,
    said: VecDeque<Exchange>,
}

/// Une phrase et ce qui y a été répondu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Exchange {
    pub phrase: String,
    pub reponse: String,
}

/// Ce qui est posé au cerveau : la phrase, et ce qui l'accompagne.
#[derive(Debug, Default)]
pub(crate) struct Question<'a> {
    pub texte: &'a str,
    pub avant: Vec<Exchange>,
    pub preferences: &'a str,
    pub contexte: &'a str,
}

/// Pourquoi le cerveau n'a pas répondu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// La limite de l'heure ou du jour est atteinte.
    Limit,
    /// Le cerveau ne répond pas, ou mal.
    NoReply,
}

/// Les demandes faites, et les limites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Usage {
    pub last_hour: usize,
    pub last_day: usize,
    pub per_hour: usize,
    pub per_day: usize,
}

#[derive(Deserialize)]
struct Reply {
    reponse: String,
}

impl Cerveau {
    /// `address` : `http://cerveau:8790` ; `key` : la clé partagée ;
    /// `per_day` : demandes par jour au plus (1 à [`MAX_PER_DAY`]).
    pub(crate) fn new(address: &str, key: &str, per_day: usize) -> Result<Self, String> {
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
        if !(1..=MAX_PER_DAY).contains(&per_day) {
            return Err(format!(
                "limite par jour du cerveau : entre 1 et {MAX_PER_DAY}"
            ));
        }
        Ok(Self {
            host,
            port,
            key: Zeroizing::new(key.to_owned()),
            per_day,
            asked: Mutex::default(),
            talks: Mutex::default(),
        })
    }

    /// Compte une demande ; une erreur (pour les logs) si une limite est
    /// atteinte.
    fn take(&self, now: Instant) -> Result<(), String> {
        let mut asked = lock(&self.asked);
        forget_older(&mut asked, now, DAY);
        if asked.len() >= self.per_day {
            return Err(format!("limite de {} demandes par jour", self.per_day));
        }
        if within(&asked, now, HOUR) >= MAX_PER_HOUR {
            return Err(format!("limite de {MAX_PER_HOUR} demandes par heure"));
        }
        asked.push_back(now);
        Ok(())
    }

    /// Les demandes de la dernière heure et du dernier jour.
    pub(crate) fn usage(&self, now: Instant) -> Usage {
        let mut asked = lock(&self.asked);
        forget_older(&mut asked, now, DAY);
        Usage {
            last_hour: within(&asked, now, HOUR),
            last_day: asked.len(),
            per_hour: MAX_PER_HOUR,
            per_day: self.per_day,
        }
    }

    /// Les derniers échanges de `user` avec le cerveau, s'ils datent de moins
    /// de [`TALK`].
    pub(crate) fn recent(&self, user: i64, now: Instant) -> Vec<Exchange> {
        let mut talks = lock(&self.talks);
        talks.retain(|_, talk| now.saturating_duration_since(talk.last) < TALK);
        talks
            .get(&user)
            .map(|talk| talk.said.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Retient un échange de `user` avec le cerveau.
    pub(crate) fn remember(&self, user: i64, phrase: &str, reply: &str, now: Instant) {
        let mut talks = lock(&self.talks);
        talks.retain(|_, talk| now.saturating_duration_since(talk.last) < TALK);
        let talk = talks.entry(user).or_insert_with(|| Talk {
            last: now,
            said: VecDeque::new(),
        });
        push(talk, phrase, reply, now);
    }

    /// Retient une réponse d'Assist, seulement si `user` parle déjà au
    /// cerveau : « allume la cuisine » entre deux phrases au cerveau fait
    /// partie de la conversation.
    pub(crate) fn note(&self, user: i64, phrase: &str, reply: &str, now: Instant) {
        let mut talks = lock(&self.talks);
        if let Some(talk) = talks
            .get_mut(&user)
            .filter(|talk| now.saturating_duration_since(talk.last) < TALK)
        {
            push(talk, phrase, reply, now);
        }
    }

    /// Oublie la conversation de `user` (« merci »).
    pub(crate) fn forget(&self, user: i64) {
        lock(&self.talks).remove(&user);
    }

    /// Pose la question au cerveau. `username` : pour les logs seulement.
    pub(crate) async fn ask(
        &self,
        username: &str,
        question: &Question<'_>,
    ) -> Result<String, Failure> {
        if let Err(limit) = self.take(Instant::now()) {
            tracing::warn!("cerveau : {limit} atteinte ({username})");
            return Err(Failure::Limit);
        }
        let start = Instant::now();
        match tokio::time::timeout(TIMEOUT, self.request(question)).await {
            Ok(Ok(reply)) => {
                tracing::info!(
                    "cerveau : réponse pour {username} en {} ms",
                    start.elapsed().as_millis()
                );
                Ok(reply)
            }
            Ok(Err(error)) => {
                tracing::warn!("cerveau : pas de réponse pour {username} ({error})");
                Err(Failure::NoReply)
            }
            Err(_) => {
                tracing::warn!("cerveau : pas de réponse pour {username} (délai dépassé)");
                Err(Failure::NoReply)
            }
        }
    }

    async fn request(&self, question: &Question<'_>) -> Result<String, String> {
        let body = body(question);
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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Retire les demandes plus vieilles que `age`.
fn forget_older(asked: &mut VecDeque<Instant>, now: Instant, age: Duration) {
    while asked
        .front()
        .is_some_and(|t| now.saturating_duration_since(*t) >= age)
    {
        asked.pop_front();
    }
}

/// Les demandes faites depuis moins de `age`.
fn within(asked: &VecDeque<Instant>, now: Instant, age: Duration) -> usize {
    asked
        .iter()
        .rev()
        .take_while(|t| now.saturating_duration_since(**t) < age)
        .count()
}

fn push(talk: &mut Talk, phrase: &str, reply: &str, now: Instant) {
    talk.said.push_back(Exchange {
        phrase: cut(phrase.trim(), MAX_TEXT),
        reponse: cut(reply.trim(), MAX_REPLY),
    });
    while talk.said.len() > MAX_AVANT {
        talk.said.pop_front();
    }
    talk.last = now;
}

/// Les `max` premiers caractères de `text`.
pub(crate) fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Le corps de `POST /demande` : la phrase, et seulement ce qui est joint.
fn body(question: &Question<'_>) -> String {
    let mut body = serde_json::Map::new();
    body.insert("texte".into(), question.texte.into());
    let skip = question.avant.len().saturating_sub(MAX_AVANT);
    let avant = &question.avant[skip..];
    if !avant.is_empty() {
        body.insert(
            "avant".into(),
            serde_json::to_value(avant).unwrap_or_default(),
        );
    }
    let preferences = cut(question.preferences.trim(), MAX_PREFERENCES);
    if !preferences.is_empty() {
        body.insert("preferences".into(), preferences.into());
    }
    let contexte = cut(question.contexte.trim(), MAX_CONTEXTE);
    if !contexte.is_empty() {
        body.insert("contexte".into(), contexte.into());
    }
    serde_json::Value::Object(body).to_string()
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
    let cut: String = joined.chars().take(MAX_REPLY - 1).collect();
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

    fn cerveau(address: &str) -> Cerveau {
        Cerveau::new(address, KEY, DEFAULT_PER_DAY).unwrap()
    }

    fn question(texte: &str) -> Question<'_> {
        Question {
            texte,
            ..Question::default()
        }
    }

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
        let new = |address: &str, key: &str| Cerveau::new(address, key, DEFAULT_PER_DAY);
        assert!(new("http://cerveau:8790", KEY).is_ok());
        assert!(new("https://cerveau:8790", KEY).is_err());
        assert!(new("cerveau", KEY).is_err());
        assert!(new("http://cerveau:8790", "courte").is_err());
        assert!(new("http://cerveau:8790", &format!("{KEY} x")).is_err());
        assert!(Cerveau::new("http://cerveau:8790", KEY, 0).is_err());
        assert!(Cerveau::new("http://cerveau:8790", KEY, MAX_PER_DAY + 1).is_err());
        // La clé n'apparaît jamais dans les logs.
        let cerveau = cerveau("http://cerveau:8790");
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
        let cerveau = cerveau("http://cerveau:8790");
        let start = Instant::now();
        for _ in 0..MAX_PER_HOUR {
            assert!(cerveau.take(start).is_ok());
        }
        assert!(cerveau.take(start + Duration::from_mins(59)).is_err());
        assert!(cerveau.take(start + HOUR).is_ok());
    }

    #[test]
    fn and_a_limit_per_day() {
        let cerveau = Cerveau::new("http://cerveau:8790", KEY, 50).unwrap();
        let start = Instant::now();
        for _ in 0..MAX_PER_HOUR {
            assert!(cerveau.take(start).is_ok());
        }
        let later = start + Duration::from_hours(2);
        for _ in MAX_PER_HOUR..50 {
            assert!(cerveau.take(later).is_ok());
        }
        // 50 en un jour : il faut attendre que les premières aient 24 h.
        let usage = cerveau.usage(later);
        assert_eq!((usage.last_hour, usage.last_day), (10, 50));
        assert_eq!((usage.per_hour, usage.per_day), (MAX_PER_HOUR, 50));
        assert!(cerveau.take(start + Duration::from_hours(23)).is_err());
        assert!(cerveau.take(start + DAY).is_ok());
        assert_eq!(cerveau.usage(start + DAY).last_day, 11);
    }

    #[test]
    fn a_conversation_lasts_a_few_minutes() {
        let cerveau = cerveau("http://cerveau:8790");
        let start = Instant::now();
        // Assist seul : rien n'est retenu tant qu'on ne parle pas au cerveau.
        cerveau.note(1, "allume le salon", "C'est fait.", start);
        assert_eq!(cerveau.recent(1, start), Vec::<Exchange>::new());
        cerveau.remember(1, "mets une ambiance cosy au salon", "C'est fait.", start);
        cerveau.note(1, "allume la cuisine", "Cuisine allumée.", start);
        let recent = cerveau.recent(1, start + Duration::from_mins(4));
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].phrase, "mets une ambiance cosy au salon");
        // Chacun sa conversation.
        assert_eq!(cerveau.recent(2, start), Vec::<Exchange>::new());
        // Trois échanges au plus, les plus récents.
        for n in 0..5 {
            cerveau.remember(1, &format!("phrase {n}"), "ok", start);
        }
        let recent = cerveau.recent(1, start);
        assert_eq!(recent.len(), MAX_AVANT);
        assert_eq!(recent[0].phrase, "phrase 2");
        // Oubliée après cinq minutes sans phrase, ou après « merci ».
        assert_eq!(cerveau.recent(1, start + TALK), Vec::<Exchange>::new());
        cerveau.remember(3, "a b", "c", start);
        cerveau.forget(3);
        assert_eq!(cerveau.recent(3, start), Vec::<Exchange>::new());
    }

    #[test]
    fn only_what_is_joined_is_sent() {
        assert_eq!(body(&question("salut")), "{\"texte\":\"salut\"}");
        let avant: Vec<Exchange> = (0..5)
            .map(|n| Exchange {
                phrase: format!("p{n}"),
                reponse: "r".into(),
            })
            .collect();
        let full = Question {
            texte: "et la chambre",
            avant,
            preferences: " cosy = 30 % ",
            contexte: &"x".repeat(MAX_CONTEXTE + 10),
        };
        let sent: serde_json::Value = serde_json::from_str(&body(&full)).unwrap();
        assert_eq!(sent["avant"].as_array().unwrap().len(), MAX_AVANT);
        assert_eq!(sent["avant"][0]["phrase"], "p2");
        assert_eq!(sent["preferences"], "cosy = 30 %");
        assert_eq!(
            sent["contexte"].as_str().unwrap().chars().count(),
            MAX_CONTEXTE
        );
    }

    #[test]
    fn replies_are_cleaned() {
        assert_eq!(clean("  Il fait\n21 degrés.\t"), "Il fait 21 degrés.");
        let long = clean(&"a ".repeat(MAX_REPLY));
        assert!(long.chars().count() <= MAX_REPLY && long.ends_with('…'));
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
        let cerveau = cerveau(&address);
        let reply = cerveau
            .ask("david", &question("quel temps fait-il dedans"))
            .await;
        assert_eq!(reply.as_deref(), Ok("Il fait 21 degrés au salon."));
        let seen = seen.await.unwrap();
        assert!(seen.starts_with("POST /demande HTTP/1.1\r\n"));
        assert!(seen.contains(&format!("Authorization: Bearer {KEY}\r\n")));
        assert!(seen.ends_with("{\"texte\":\"quel temps fait-il dedans\"}"));
    }

    #[tokio::test]
    async fn no_reply_when_the_cerveau_fails() {
        let (address, _seen) = fake("HTTP/1.1 503 Service Unavailable\r\n\r\n{}").await;
        let cerveau = cerveau(&address);
        assert_eq!(
            cerveau.ask("david", &question("fais un truc")).await,
            Err(Failure::NoReply)
        );
        // Personne n'écoute : pas de réponse non plus.
        let closed = Cerveau::new("http://127.0.0.1:9", KEY, 1).unwrap();
        assert_eq!(
            closed.ask("david", &question("fais un truc")).await,
            Err(Failure::NoReply)
        );
        // La limite du jour (1) est atteinte : le cerveau n'est même pas appelé.
        assert_eq!(
            closed.ask("david", &question("fais un truc")).await,
            Err(Failure::Limit)
        );
    }
}
