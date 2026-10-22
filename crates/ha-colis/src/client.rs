//! La boucle des colis : toutes les [`POLL`], lire les nouveaux e-mails
//! d'Amazon, en déduire l'état des colis, et le publier.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ha_core::{Attributes, Background, CancellationToken, Core, EntityId, Health};
use ha_storage::{MailAccount, Store};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use crate::ColisError;
use crate::amazon::{self, Notice, Parcel, Step};
use crate::imap::Session;
use crate::mail::{self, Headers};

/// La boîte est relue à ce rythme.
pub const POLL: Duration = Duration::from_mins(3);
/// Les e-mails de ces derniers jours comptent.
const DAYS: i64 = 14;
/// Au plus ce nombre d'e-mails d'Amazon lus (les plus récents).
const MAX_MAILS: usize = 60;
/// Au plus ce nombre de colis dans les attributs.
const MAX_PARCELS: usize = 10;
/// Un relevé entier ne dure pas plus.
const TIMEOUT: Duration = Duration::from_secs(90);
/// Une photo d'article fait au plus cette taille.
const MAX_IMAGE: usize = 2 << 20;
const MAX_BACKOFF: Duration = Duration::from_mins(15);
const UNAUTHORIZED_WAIT: Duration = Duration::from_mins(30);
/// Un évènement n'est lancé que pour un e-mail aussi récent (pas pour un
/// vieil e-mail découvert plus tard).
const FRESH: Duration = Duration::from_hours(6);

/// Où lire les e-mails : la vraie boîte (IMAP sur TLS), ou, pour les
/// essais, un faux serveur sur ce PC.
#[derive(Debug, Clone)]
pub struct Mailbox {
    test: Option<SocketAddr>,
    images: Option<String>,
    poll: Duration,
}

impl Mailbox {
    /// La vraie boîte, à l'adresse du compte.
    #[must_use]
    pub const fn real() -> Self {
        Self {
            test: None,
            images: None,
            poll: POLL,
        }
    }

    /// Un faux serveur IMAP (sans chiffrement) **sur ce PC seulement**, et
    /// éventuellement un faux serveur d'images (`http://127.0.0.1:…`). Il
    /// est relu toutes les 2 secondes.
    #[must_use]
    pub fn local(imap: &str, images: Option<&str>) -> Option<Self> {
        let address: SocketAddr = imap.parse().ok()?;
        if !address.ip().is_loopback() {
            return None;
        }
        let images = images
            .filter(|url| url.starts_with("http://127.0.0.1:") && !url[17..].contains('@'))
            .map(|url| url.trim_end_matches('/').to_owned());
        Some(Self {
            test: Some(address),
            images,
            poll: Duration::from_secs(2),
        })
    }

    async fn connect(&self, account: &MailAccount) -> Result<Session<Box<dyn Stream>>, ColisError> {
        let stream: Box<dyn Stream> = if let Some(address) = self.test {
            Box::new(TcpStream::connect(address).await.map_err(ColisError::Io)?)
        } else {
            if !ha_tls::is_valid_host(&account.host) || account.host.contains(':') {
                return Err(ColisError::Refused("adresse du serveur invalide".into()));
            }
            let tcp = TcpStream::connect((account.host.as_str(), account.port))
                .await
                .map_err(ColisError::Io)?;
            let config = ha_tls::public_config().map_err(|e| ColisError::Tls(e.to_string()))?;
            let name = rustls::pki_types::ServerName::try_from(account.host.clone())
                .map_err(|e| ColisError::Tls(e.to_string()))?;
            let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect(name, tcp)
                .await
                .map_err(|e| ColisError::Tls(e.to_string()))?;
            Box::new(tls)
        };
        Session::start(stream).await
    }

    fn http(&self) -> Result<reqwest::Client, ColisError> {
        ha_tls::web_client(self.images.is_none()).map_err(|e| ColisError::Tls(e.to_string()))
    }

    /// L'adresse où télécharger une photo d'Amazon (le faux serveur, pour
    /// les essais).
    fn image_url(&self, url: &str) -> String {
        match (&self.images, url.split_once(".com/")) {
            (Some(base), Some((_, path))) => format!("{base}/{path}"),
            _ => url.to_owned(),
        }
    }
}

/// Une connexion : TLS, ou TCP pour les essais.
trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// Ce qui est gardé d'un relevé à l'autre : les e-mails déjà lus.
#[derive(Debug, Default)]
struct Cache {
    folder: Option<String>,
    validity: u64,
    notices: HashMap<u32, Seen>,
}

impl Cache {
    fn notices(&self) -> Vec<Notice> {
        self.notices
            .values()
            .filter_map(|seen| match seen {
                Seen::Parcel(notice) => Some(notice.clone()),
                Seen::Other | Seen::Unsigned => None,
            })
            .collect()
    }
}

/// Ce qu'on a tiré d'un e-mail.
#[derive(Debug, Clone)]
enum Seen {
    Parcel(Notice),
    /// D'Amazon, mais pas un colis (publicité, facture…), ou illisible.
    Other,
    /// Pas de signature d'Amazon vérifiée : ignoré.
    Unsigned,
}

/// Un e-mail lu, compris.
fn understand(uid: u32, headers: &[u8], text: &[u8]) -> Seen {
    let headers = Headers::parse(headers);
    if !headers.is_from_amazon() {
        debug!("colis : e-mail {uid} ignoré (pas signé par Amazon)");
        return Seen::Unsigned;
    }
    let Some(step) = amazon::classify(&headers.subject) else {
        return Seen::Other;
    };
    let text = mail::readable_text(text);
    Seen::Parcel(Notice {
        uid,
        step,
        article: amazon::article(&headers.subject),
        order: mail::order_number(&text),
        image: mail::product_image(&text),
        at: parse_date(&headers.date).unwrap_or_else(Timestamp::now),
    })
}

/// `Wed, 22 Oct 2026 07:15:32 +0000 (UTC)`.
fn parse_date(date: &str) -> Option<Timestamp> {
    let date = date.split(" (").next().unwrap_or(date).trim();
    jiff::fmt::rfc2822::parse(date)
        .ok()
        .map(|zoned| zoned.timestamp())
}

/// Lit les e-mails d'Amazon des derniers jours (seuls les nouveaux sont
/// téléchargés).
async fn read_mail(
    mailbox: &Mailbox,
    account: &MailAccount,
    cache: &mut Cache,
) -> Result<Vec<Notice>, ColisError> {
    let mut session = mailbox.connect(account).await?;
    session
        .login(&account.username, account.password.as_str())
        .await?;
    let folder = if let Some(folder) = &cache.folder {
        folder.clone()
    } else {
        let folder = session.all_mail().await?;
        cache.folder = Some(folder.clone());
        folder
    };
    let validity = session.examine(&folder).await?;
    if validity != cache.validity {
        cache.notices.clear();
        cache.validity = validity;
    }
    let since = Zoned::now()
        .checked_sub(DAYS.days())
        .map_err(|e| ColisError::Protocol(e.to_string()))?;
    let since = since.strftime("%-d-%b-%Y").to_string();
    let mut uids = session.search(&since, "amazon").await?;
    if uids.len() > MAX_MAILS {
        uids.drain(..uids.len() - MAX_MAILS);
    }
    cache.notices.retain(|uid, _| uids.contains(uid));
    for &uid in &uids {
        if cache.notices.contains_key(&uid) {
            continue;
        }
        let seen = session
            .fetch(uid)
            .await?
            .map_or(Seen::Other, |(headers, text)| {
                understand(uid, &headers, &text)
            });
        cache.notices.insert(uid, seen);
    }
    session.logout().await;
    Ok(cache.notices())
}

/// Ce que voit `ha-server colis pair` et `status`.
#[derive(Debug, Clone)]
pub struct Report {
    /// Les colis des deux dernières semaines.
    pub parcels: Vec<Parcel>,
    /// Les e-mails « d'Amazon » des deux dernières semaines.
    pub mails: usize,
    /// Ceux qui ont été ignorés : pas de signature d'Amazon vérifiée (un
    /// faux, ou un transfert qui l'a cassée).
    pub unsigned: usize,
}

/// Vérifie le compte et lit les colis, pour `ha-server colis pair` et
/// `status`.
pub async fn check(mailbox: &Mailbox, account: &MailAccount) -> Result<Report, ColisError> {
    let mut cache = Cache::default();
    let notices = tokio::time::timeout(TIMEOUT, read_mail(mailbox, account, &mut cache))
        .await
        .map_err(|_| ColisError::Timeout)??;
    Ok(Report {
        parcels: amazon::parcels(&notices),
        mails: cache.notices.len(),
        unsigned: cache
            .notices
            .values()
            .filter(|seen| matches!(seen, Seen::Unsigned))
            .count(),
    })
}

/// L'intégration des colis, en marche.
#[derive(Debug)]
pub struct Colis(Background);

impl Colis {
    /// Démarre (la boîte doit être associée : `ha-server colis pair`).
    #[must_use]
    pub fn start(store: Store, mailbox: Mailbox, core: Arc<Core>) -> Self {
        let supervisor = Arc::clone(&core);
        Self(ha_core::supervise(
            supervisor,
            crate::ID,
            crate::NAME,
            move |token| run(store.clone(), mailbox.clone(), Arc::clone(&core), token),
        ))
    }

    pub async fn stop(self) {
        self.0.stop().await;
    }
}

async fn run(store: Store, mailbox: Mailbox, core: Arc<Core>, token: CancellationToken) {
    let http = match mailbox.http() {
        Ok(http) => http,
        Err(error) => {
            warn!("colis : impossible de préparer la connexion : {error}");
            return;
        }
    };
    let mut cache = Cache::default();
    let mut published = Published::default();
    let mut backoff = Duration::from_mins(1);
    loop {
        let reading = async {
            let account = store
                .mail_account()
                .await?
                .ok_or(ColisError::Unauthorized)?;
            tokio::time::timeout(TIMEOUT, read_mail(&mailbox, &account, &mut cache))
                .await
                .map_err(|_| ColisError::Timeout)?
        };
        // L'arrêt du serveur n'attend pas la fin d'un relevé (une boîte
        // injoignable peut le faire durer 90 s).
        let result = tokio::select! {
            result = reading => result,
            () = token.cancelled() => break,
        };
        let wait = match result {
            Ok(notices) => {
                let parcels = amazon::parcels(&notices);
                tokio::select! {
                    () = published.publish(&core, &parcels, &mailbox, &http) => {}
                    () = token.cancelled() => break,
                }
                core.report(crate::ID, crate::NAME, Health::Connected);
                backoff = Duration::from_mins(1);
                mailbox.poll
            }
            Err(error) => {
                cache.folder = None;
                let (wait, message) = match &error {
                    ColisError::Unauthorized => (
                        UNAUTHORIZED_WAIT,
                        "compte refusé : refais « ha-server colis pair »".to_owned(),
                    ),
                    _ => (
                        if mailbox.test.is_some() {
                            Duration::from_secs(5)
                        } else {
                            backoff
                        },
                        format!("boîte aux lettres injoignable ({error})"),
                    ),
                };
                warn!("colis : {message}, nouvel essai dans {} s", wait.as_secs());
                core.report(
                    crate::ID,
                    crate::NAME,
                    Health::Retrying {
                        error: message,
                        retry_at: SystemTime::now() + wait,
                    },
                );
                backoff = (backoff * 2).min(MAX_BACKOFF);
                wait
            }
        };
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = token.cancelled() => break,
        }
    }
    info!("colis : arrêté");
}

/// Ce qui a déjà été publié : l'état de chaque colis (pour ne lancer un
/// évènement qu'au changement), et la photo.
#[derive(Debug, Default)]
struct Published {
    started: bool,
    steps: HashMap<String, Step>,
    image: Option<String>,
    image_failed: Option<String>,
    camera_cleared: bool,
}

fn millis(at: Timestamp) -> i64 {
    at.as_millisecond()
}

fn local_time(at: Timestamp) -> String {
    at.to_zoned(TimeZone::system())
        .strftime("%H:%M")
        .to_string()
}

/// Le colis en attributs.
fn describe(parcel: &Parcel) -> Value {
    let name = amazon::short_name(&parcel.article);
    json!({
        "id": parcel.id,
        "etat": parcel.step.key(),
        "article": name,
        "article_complet": parcel.article,
        "phrase": parcel.step.sentence(&name),
        "heure": local_time(parcel.at),
        "a": millis(parcel.at),
    })
}

/// Les colis à montrer dans une fenêtre : en livraison, livrés, ou en
/// difficulté.
const fn worth_a_popup(step: Step) -> bool {
    matches!(step, Step::OutForDelivery | Step::Delivered | Step::Problem)
}

impl Published {
    async fn publish(
        &mut self,
        core: &Core,
        parcels: &[Parcel],
        mailbox: &Mailbox,
        http: &reqwest::Client,
    ) {
        let zone = TimeZone::system();
        let today = Timestamp::now().to_zoned(zone.clone()).date();
        let is_today = |at: Timestamp| at.to_zoned(zone.clone()).date() == today;
        let on_the_way = parcels.iter().filter(|p| p.step.on_the_way()).count();
        let arriving = parcels
            .iter()
            .filter(|p| p.step == Step::OutForDelivery && is_today(p.at))
            .count();
        let delivered = parcels
            .iter()
            .filter(|p| p.step == Step::Delivered && is_today(p.at))
            .count();
        let latest = parcels.iter().rev().find(|p| worth_a_popup(p.step));

        // Les évènements, au changement (pas au premier relevé).
        let fresh = |at: Timestamp| {
            Timestamp::now()
                .duration_since(at)
                .try_into()
                .is_ok_and(|age: Duration| age < FRESH)
        };
        for parcel in parcels {
            let before = self.steps.insert(parcel.id.clone(), parcel.step);
            if !self.started || before == Some(parcel.step) || !fresh(parcel.at) {
                continue;
            }
            let event = match parcel.step {
                Step::OutForDelivery => crate::OUT_FOR_DELIVERY,
                Step::Delivered => crate::DELIVERED,
                _ => continue,
            };
            info!(
                "colis : {}",
                parcel.step.sentence(&amazon::short_name(&parcel.article))
            );
            let mut data = Attributes::new();
            if let Value::Object(fields) = describe(parcel) {
                data.extend(fields);
            }
            if let Err(error) = core.fire(event, data) {
                warn!("colis : évènement refusé ({error})");
            }
        }
        self.steps
            .retain(|id, _| parcels.iter().any(|p| &p.id == id));
        self.started = true;

        // La photo du dernier colis.
        let mut image_of = None;
        if let Some(parcel) = latest
            && let Some(url) = &parcel.image
        {
            if self.image.as_ref() == Some(url) {
                image_of = Some(parcel.id.clone());
            } else if self.image_failed.as_ref() != Some(url) {
                match download(http, &mailbox.image_url(url)).await {
                    Ok(bytes) => match EntityId::new(crate::CAMERA)
                        .map_err(|e| e.to_string())
                        .and_then(|id| core.images.set(id, &bytes).map(|_| ()))
                    {
                        Ok(()) => {
                            self.image = Some(url.clone());
                            image_of = Some(parcel.id.clone());
                            let mut attributes = Attributes::new();
                            attributes.insert("friendly_name".into(), json!(crate::NAME));
                            attributes.insert("colis".into(), json!(parcel.id));
                            let at = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
                            attributes.insert("image_at".into(), json!(at));
                            if let Ok(id) = EntityId::new(crate::CAMERA)
                                && let Err(error) =
                                    core.states.set_by(crate::ID, id, "idle", attributes)
                            {
                                warn!("colis : photo refusée ({error})");
                            }
                        }
                        Err(error) => {
                            warn!("colis : photo refusée ({error})");
                            self.image_failed = Some(url.clone());
                        }
                    },
                    Err(error) => {
                        warn!("colis : photo introuvable ({error})");
                        self.image_failed = Some(url.clone());
                    }
                }
            }
        }

        // Une photo d'avant le redémarrage (l'état est restauré, pas l'image) :
        // on l'efface, pour ne pas demander une image qui n'existe plus.
        if self.image.is_none()
            && !self.camera_cleared
            && let Ok(id) = EntityId::new(crate::CAMERA)
            && core.states.get(&id).is_some()
        {
            let mut attributes = Attributes::new();
            attributes.insert("friendly_name".into(), json!(crate::NAME));
            if let Err(error) = core.states.set_by(crate::ID, id, "idle", attributes) {
                warn!("colis : photo refusée ({error})");
            }
        }
        self.camera_cleared = true;

        let mut attributes = Attributes::new();
        attributes.insert("friendly_name".into(), json!(crate::NAME));
        attributes.insert("icon".into(), json!("mdi:package-variant"));
        attributes.insert("unit_of_measurement".into(), json!("colis"));
        attributes.insert("aujourdhui".into(), json!(arriving));
        attributes.insert("livres_aujourdhui".into(), json!(delivered));
        attributes.insert(
            "colis".into(),
            Value::Array(
                parcels
                    .iter()
                    .rev()
                    .take(MAX_PARCELS)
                    .map(describe)
                    .collect(),
            ),
        );
        if let Some(parcel) = latest {
            let mut last = describe(parcel);
            if let Value::Object(fields) = &mut last {
                fields.insert("image".into(), json!(image_of.as_ref() == Some(&parcel.id)));
            }
            attributes.insert("dernier".into(), last);
        }
        match EntityId::new(crate::SENSOR) {
            Ok(id) => {
                if let Err(error) =
                    core.states
                        .set_by(crate::ID, id, on_the_way.to_string(), attributes)
                {
                    warn!("colis : état refusé ({error})");
                }
            }
            Err(error) => warn!("colis : {error}"),
        }
    }
}

/// Télécharge une photo d'article (2 Mio au plus).
async fn download(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut response = http
        .get(url)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("réponse {}", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if bytes.len() + chunk.len() > MAX_IMAGE {
            return Err("photo trop grosse".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn mail_dates() {
        let at = parse_date("Wed, 21 Oct 2026 07:15:32 +0000 (UTC)").unwrap();
        assert_eq!(at.to_string(), "2026-10-21T07:15:32Z");
        assert!(parse_date("hier").is_none());
    }

    #[test]
    fn only_local_test_servers() {
        assert!(Mailbox::local("127.0.0.1:1430", None).is_some());
        assert!(Mailbox::local("192.168.1.2:143", None).is_none());
        let mailbox = Mailbox::local("127.0.0.1:1430", Some("http://127.0.0.1:9000/")).unwrap();
        assert_eq!(
            mailbox.image_url("https://m.media-amazon.com/images/I/a.jpg"),
            "http://127.0.0.1:9000/images/I/a.jpg"
        );
        let mailbox = Mailbox::local("127.0.0.1:1430", Some("http://pirate.example/")).unwrap();
        assert_eq!(
            mailbox.image_url("https://m.media-amazon.com/images/I/a.jpg"),
            "https://m.media-amazon.com/images/I/a.jpg"
        );
    }
}
