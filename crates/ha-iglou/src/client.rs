//! Le client : l'API REST de Firebase, comme l'appli Iglou elle-même.
//!
//! 1. **Connexion** (Firebase Authentication) : l'adresse et le mot de passe
//!    du compte de la maison donnent un jeton valable une heure, et
//!    l'identifiant du compte (`uid`).
//! 2. **Lecture et écriture** (Firestore) : avec ce jeton, comme n'importe
//!    quel membre. Les règles de sécurité de l'appli s'appliquent : le compte
//!    ne lit et n'écrit que les maisons dont il est membre.
//!
//! Chaque identifiant remis dans une adresse est vérifié avant
//! ([`crate::firestore::is_id`]) : une réponse ne peut pas nous faire lire
//! ailleurs.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use ha_storage::IglouAccount;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::firestore::{self, Document, is_id};
use crate::list::{Aliment, Appareil, Item, Libre, MAX_NAME, shopping_list};

/// L'API de connexion de Firebase.
pub const AUTH: &str = "https://identitytoolkit.googleapis.com/v1";
/// L'API de Firestore.
pub const FIRESTORE: &str = "https://firestore.googleapis.com/v1";
/// Taille d'une réponse au plus.
const MAX_BYTES: usize = 4 << 20;
/// Pages lues au plus par collection (300 documents chacune).
const MAX_PAGES: usize = 10;
const PAGE_SIZE: u32 = 300;
/// Le jeton vaut une heure : on en reprend un avant.
const TOKEN_LIFE: Duration = Duration::from_mins(50);
/// Articles dans la liste au plus (au-delà, on n'en ajoute plus).
pub const MAX_ITEMS: usize = 500;

/// Ce qui peut mal se passer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IglouError {
    #[error("adresse e-mail ou mot de passe Iglou refusé")]
    Credentials,
    #[error("Iglou demande d'attendre avant de se reconnecter")]
    RateLimited,
    #[error("Iglou injoignable")]
    Unreachable,
    #[error("refusé par Iglou : le compte est-il encore membre de la maison ?")]
    Denied,
    #[error("{0}")]
    Invalid(String),
    #[error("réponse inattendue d'Iglou ({0})")]
    Unexpected(String),
}

impl IglouError {
    /// Faut-il attendre longtemps avant de réessayer ?
    #[must_use]
    pub fn is_lasting(&self) -> bool {
        matches!(self, Self::Credentials | Self::RateLimited | Self::Denied)
    }
}

/// Les adresses des deux API (d'autres pour les essais).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub auth: String,
    pub firestore: String,
}

impl Endpoints {
    /// Les vraies adresses de Google.
    #[must_use]
    pub fn google() -> Self {
        Self {
            auth: AUTH.to_owned(),
            firestore: FIRESTORE.to_owned(),
        }
    }

    /// Un faux Firebase des essais, sur ce PC (`http://127.0.0.1:…`).
    #[must_use]
    pub fn local(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            auth: format!("{base}/v1"),
            firestore: format!("{base}/v1"),
        }
    }
}

/// Le projet Firebase : minuscules, chiffres et tirets.
#[must_use]
pub fn is_project_id(id: &str) -> bool {
    (4..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// La clé « web » de Firebase (`AIza…`).
#[must_use]
pub fn is_api_key(key: &str) -> bool {
    (20..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Un code d'invitation Iglou : 6 lettres majuscules ou chiffres.
#[must_use]
pub fn normalize_code(code: &str) -> Option<String> {
    let code: String = code
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    (code.len() == 6 && code.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(code)
}

struct Session {
    token: Zeroizing<String>,
    uid: String,
    at: Instant,
}

/// Le client.
pub struct Client {
    http: reqwest::Client,
    endpoints: Endpoints,
    project_id: String,
    api_key: String,
    email: String,
    password: Zeroizing<String>,
    session: Mutex<Option<Session>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("project_id", &self.project_id)
            .field("email", &self.email)
            .finish_non_exhaustive()
    }
}

/// Une maison Iglou dont le compte est membre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Maison {
    pub id: String,
    pub nom: String,
}

impl Client {
    /// Vérifie le projet et la clé, sans encore se connecter.
    pub fn new(
        http: reqwest::Client,
        endpoints: Endpoints,
        project_id: &str,
        api_key: &str,
        email: &str,
        password: Zeroizing<String>,
    ) -> Result<Self, IglouError> {
        if !is_project_id(project_id) {
            return Err(IglouError::Invalid(
                "projet Firebase invalide (minuscules, chiffres et tirets)".into(),
            ));
        }
        if !is_api_key(api_key) {
            return Err(IglouError::Invalid("clé web Firebase invalide".into()));
        }
        Ok(Self {
            http,
            endpoints,
            project_id: project_id.to_owned(),
            api_key: api_key.to_owned(),
            email: email.to_owned(),
            password,
            session: Mutex::new(None),
        })
    }

    /// Le client du compte enregistré.
    pub fn from_account(
        http: reqwest::Client,
        endpoints: Endpoints,
        account: &IglouAccount,
    ) -> Result<Self, IglouError> {
        Self::new(
            http,
            endpoints,
            &account.project_id,
            &account.api_key,
            &account.email,
            account.password.clone(),
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Se connecte : un jeton neuf et l'identifiant du compte.
    async fn sign_in(&self) -> Result<(Zeroizing<String>, String), IglouError> {
        let body = Zeroizing::new(
            json!({
                "email": self.email,
                "password": self.password.as_str(),
                "returnSecureToken": true,
            })
            .to_string(),
        );
        let response = self
            .http
            .post(format!(
                "{}/accounts:signInWithPassword?key={}",
                self.endpoints.auth, self.api_key
            ))
            .header("Content-Type", "application/json")
            .body(body.as_bytes().to_vec())
            .send()
            .await
            .map_err(|_| IglouError::Unreachable)?;
        let status = response.status().as_u16();
        let text = Zeroizing::new(read(response).await?);
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let message = json
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(
                if message.starts_with("TOO_MANY_ATTEMPTS") || status == 429 {
                    IglouError::RateLimited
                } else if message.contains("API key") || message.contains("API_KEY") {
                    IglouError::Invalid("clé web Firebase refusée".into())
                } else if status == 400 {
                    IglouError::Credentials
                } else {
                    IglouError::Unexpected(format!("connexion : {status}"))
                },
            );
        }
        let token = json
            .get("idToken")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty() && t.len() < 8192)
            .ok_or_else(|| IglouError::Unexpected("pas de jeton reçu".into()))?;
        let uid = json
            .get("localId")
            .and_then(Value::as_str)
            .filter(|uid| is_id(uid))
            .ok_or_else(|| IglouError::Unexpected("compte sans identifiant".into()))?;
        Ok((Zeroizing::new(token.to_owned()), uid.to_owned()))
    }

    /// Le jeton en cours, ou un neuf.
    async fn token(&self) -> Result<(Zeroizing<String>, String), IglouError> {
        if let Some(session) = self.lock().as_ref()
            && session.at.elapsed() < TOKEN_LIFE
        {
            return Ok((session.token.clone(), session.uid.clone()));
        }
        let (token, uid) = self.sign_in().await?;
        *self.lock() = Some(Session {
            token: token.clone(),
            uid: uid.clone(),
            at: Instant::now(),
        });
        Ok((token, uid))
    }

    /// L'identifiant du compte (en se connectant au besoin).
    pub async fn uid(&self) -> Result<String, IglouError> {
        Ok(self.token().await?.1)
    }

    fn documents(&self) -> String {
        format!(
            "{}/projects/{}/databases/(default)/documents",
            self.endpoints.firestore, self.project_id
        )
    }

    /// Le nom complet d'un document, pour une écriture.
    fn name(&self, path: &str) -> String {
        format!(
            "projects/{}/databases/(default)/documents/{path}",
            self.project_id
        )
    }

    /// Une requête avec le jeton ; un jeton refusé est refait une fois.
    /// `None` : le document n'existe pas.
    async fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>, IglouError> {
        for attempt in 0..2 {
            let (token, _) = self.token().await?;
            let mut request = self
                .http
                .request(method.clone(), url)
                .bearer_auth(token.as_str());
            if let Some(body) = body {
                request = request
                    .header("Content-Type", "application/json")
                    .body(body.to_string());
            }
            let response = request.send().await.map_err(|_| IglouError::Unreachable)?;
            let status = response.status().as_u16();
            if status == 401 && attempt == 0 {
                *self.lock() = None;
                continue;
            }
            return match status {
                200..=299 => {
                    let text = read(response).await?;
                    serde_json::from_str(&text)
                        .map(Some)
                        .map_err(|_| IglouError::Unexpected("réponse illisible".into()))
                }
                404 => Ok(None),
                403 => Err(IglouError::Denied),
                429 => Err(IglouError::RateLimited),
                _ => Err(IglouError::Unexpected(format!("{status}"))),
            };
        }
        Err(IglouError::Credentials)
    }

    /// Tous les documents d'une collection (`maisons/<id>/aliments`).
    async fn list(&self, path: &str) -> Result<Vec<Document>, IglouError> {
        let mut documents = Vec::new();
        let mut page: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut url = url::Url::parse(&format!("{}/{path}", self.documents()))
                .map_err(|_| IglouError::Unexpected("adresse".into()))?;
            url.query_pairs_mut()
                .append_pair("pageSize", &PAGE_SIZE.to_string());
            if let Some(token) = &page {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let Some(json) = self
                .request(reqwest::Method::GET, url.as_str(), None)
                .await?
            else {
                break;
            };
            documents.extend(
                json.get("documents")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Document::from_json),
            );
            match json.get("nextPageToken").and_then(Value::as_str) {
                Some(next) if !next.is_empty() && next.len() < 4096 => page = Some(next.to_owned()),
                _ => break,
            }
        }
        Ok(documents)
    }

    async fn get(&self, path: &str) -> Result<Option<Document>, IglouError> {
        let url = format!("{}/{path}", self.documents());
        Ok(self
            .request(reqwest::Method::GET, &url, None)
            .await?
            .as_ref()
            .and_then(Document::from_json))
    }

    /// Des écritures d'un seul coup (toutes, ou aucune).
    async fn commit(&self, writes: Vec<Value>) -> Result<(), IglouError> {
        let url = format!("{}:commit", self.documents());
        self.request(
            reqwest::Method::POST,
            &url,
            Some(&json!({ "writes": writes })),
        )
        .await?;
        Ok(())
    }

    /// Les maisons dont le compte est membre.
    pub async fn maisons(&self) -> Result<Vec<Maison>, IglouError> {
        let uid = self.uid().await?;
        let url = format!("{}:runQuery", self.documents());
        let query = json!({
            "structuredQuery": {
                "from": [{ "collectionId": "membres", "allDescendants": true }],
                "where": { "fieldFilter": {
                    "field": { "fieldPath": "uid" },
                    "op": "EQUAL",
                    "value": firestore::string(&uid),
                }},
                "limit": 20,
            }
        });
        let rows = self
            .request(reqwest::Method::POST, &url, Some(&query))
            .await?
            .unwrap_or(Value::Null);
        let mut maisons = Vec::new();
        for row in rows.as_array().into_iter().flatten() {
            let Some(member) = row.get("document").and_then(Document::from_json) else {
                continue;
            };
            let Some(id) = member.text("maisonId").filter(|id| is_id(id)) else {
                continue;
            };
            let nom = self
                .get(&format!("maisons/{id}"))
                .await?
                .and_then(|m| m.text("nom").map(|n| clean(n, 60)))
                .unwrap_or_else(|| "Maison".to_owned());
            if !maisons.iter().any(|m: &Maison| m.id == id) {
                maisons.push(Maison {
                    id: id.to_owned(),
                    nom,
                });
            }
        }
        Ok(maisons)
    }

    /// Rejoint la maison du code d'invitation (comme « Rejoindre » dans
    /// l'appli), sous le prénom donné. Rend la maison.
    pub async fn join(&self, code: &str, prenom: &str) -> Result<Maison, IglouError> {
        let code = normalize_code(code)
            .ok_or_else(|| IglouError::Invalid("le code doit avoir 6 caractères".into()))?;
        let uid = self.uid().await?;
        let invitation = self
            .get(&format!("invitations/{code}"))
            .await?
            .ok_or_else(|| IglouError::Invalid("code d'invitation inconnu ou expiré".into()))?;
        let maison = invitation
            .text("maisonId")
            .filter(|id| is_id(id))
            .ok_or_else(|| IglouError::Unexpected("invitation sans maison".into()))?
            .to_owned();
        if !self.maisons().await?.iter().any(|m| m.id == maison) {
            let prenom = clean(prenom, 40);
            let write = json!({
                "update": {
                    "name": self.name(&format!("maisons/{maison}/membres/{uid}")),
                    "fields": {
                        "uid": firestore::string(&uid),
                        "maisonId": firestore::string(&maison),
                        "prenom": firestore::string(if prenom.is_empty() { "Maison" } else { &prenom }),
                        "role": firestore::string("membre"),
                        "code": firestore::string(&code),
                    },
                },
                "currentDocument": { "exists": false },
                "updateTransforms": [{ "fieldPath": "joinedAt", "setToServerValue": "REQUEST_TIME" }],
            });
            self.commit(vec![write])
                .await
                .map_err(|error| match error {
                    IglouError::Denied => IglouError::Invalid("code d'invitation refusé".into()),
                    other => other,
                })?;
        }
        let nom = self
            .get(&format!("maisons/{maison}"))
            .await?
            .and_then(|m| m.text("nom").map(|n| clean(n, 60)))
            .unwrap_or_else(|| "Maison".to_owned());
        Ok(Maison { id: maison, nom })
    }

    /// La liste de courses de la maison.
    pub async fn shopping_list(&self, maison: &str) -> Result<Vec<Item>, IglouError> {
        check_id(maison)?;
        let base = format!("maisons/{maison}");
        let aliments: Vec<Aliment> = self
            .list(&format!("{base}/aliments"))
            .await?
            .iter()
            .filter_map(Aliment::from_document)
            .collect();
        let appareils: Vec<Appareil> = self
            .list(&format!("{base}/appareils"))
            .await?
            .iter()
            .filter_map(Appareil::from_document)
            .collect();
        let libres: Vec<Libre> = self
            .list(&format!("{base}/coursesLibres"))
            .await?
            .iter()
            .filter_map(Libre::from_document)
            .collect();
        Ok(shopping_list(&aliments, &appareils, &libres))
    }

    /// Ajoute un article libre (« Pain »), une pièce.
    pub async fn add(&self, maison: &str, nom: &str) -> Result<(), IglouError> {
        check_id(maison)?;
        let nom = clean(nom, MAX_NAME);
        if nom.is_empty() {
            return Err(IglouError::Invalid("article vide".into()));
        }
        let uid = self.uid().await?;
        let id = new_id()?;
        let write = json!({
            "update": {
                "name": self.name(&format!("maisons/{maison}/coursesLibres/{id}")),
                "fields": {
                    "nom": firestore::string(&nom),
                    "quantite": firestore::number(1.0),
                    "unite": firestore::string("pièce"),
                    "dansPanier": firestore::boolean(false),
                    "ajoutePar": firestore::string(&uid),
                },
            },
            "currentDocument": { "exists": false },
            "updateTransforms": [{ "fieldPath": "createdAt", "setToServerValue": "REQUEST_TIME" }],
        });
        self.commit(vec![write]).await
    }

    /// Coche (dans le panier) ou décoche un article.
    pub async fn set_in_cart(
        &self,
        maison: &str,
        uid: &str,
        in_cart: bool,
    ) -> Result<(), IglouError> {
        check_id(maison)?;
        let write = match parse_uid(uid)? {
            (Kind::Aliment, id) => {
                // Les règles de l'appli veulent savoir qui modifie un aliment.
                let me = self.uid().await?;
                json!({
                    "update": {
                        "name": self.name(&format!("maisons/{maison}/aliments/{id}")),
                        "fields": {
                            "dansPanier": firestore::boolean(in_cart),
                            "modifiePar": firestore::string(&me),
                        },
                    },
                    "updateMask": { "fieldPaths": ["dansPanier", "modifiePar"] },
                    "currentDocument": { "exists": true },
                    "updateTransforms": [{ "fieldPath": "updatedAt", "setToServerValue": "REQUEST_TIME" }],
                })
            }
            (Kind::Libre, id) => json!({
                "update": {
                    "name": self.name(&format!("maisons/{maison}/coursesLibres/{id}")),
                    "fields": { "dansPanier": firestore::boolean(in_cart) },
                },
                "updateMask": { "fieldPaths": ["dansPanier"] },
                "currentDocument": { "exists": true },
            }),
        };
        self.commit(vec![write]).await
    }

    /// Renomme un article libre.
    pub async fn rename(&self, maison: &str, uid: &str, nom: &str) -> Result<(), IglouError> {
        check_id(maison)?;
        let (Kind::Libre, id) = parse_uid(uid)? else {
            return Err(IglouError::Invalid(
                "un aliment du frigo se renomme dans l'appli Iglou".into(),
            ));
        };
        let nom = clean(nom, MAX_NAME);
        if nom.is_empty() {
            return Err(IglouError::Invalid("nom vide".into()));
        }
        self.commit(vec![json!({
            "update": {
                "name": self.name(&format!("maisons/{maison}/coursesLibres/{id}")),
                "fields": { "nom": firestore::string(&nom) },
            },
            "updateMask": { "fieldPaths": ["nom"] },
            "currentDocument": { "exists": true },
        })])
        .await
    }

    /// Retire un article libre de la liste.
    pub async fn remove(&self, maison: &str, uid: &str) -> Result<(), IglouError> {
        check_id(maison)?;
        let (Kind::Libre, id) = parse_uid(uid)? else {
            return Err(IglouError::Invalid(
                "un aliment du frigo quitte la liste quand il est rangé (dans l'appli Iglou)"
                    .into(),
            ));
        };
        self.commit(vec![json!({
            "delete": self.name(&format!("maisons/{maison}/coursesLibres/{id}")),
        })])
        .await
    }
}

enum Kind {
    Aliment,
    Libre,
}

/// `a:<id>` ou `l:<id>`.
fn parse_uid(uid: &str) -> Result<(Kind, &str), IglouError> {
    let (kind, id) = match uid.split_once(':') {
        Some(("a", id)) => (Kind::Aliment, id),
        Some(("l", id)) => (Kind::Libre, id),
        _ => return Err(IglouError::Invalid("article inconnu".into())),
    };
    if !is_id(id) {
        return Err(IglouError::Invalid("article inconnu".into()));
    }
    Ok((kind, id))
}

fn check_id(id: &str) -> Result<(), IglouError> {
    if is_id(id) {
        Ok(())
    } else {
        Err(IglouError::Invalid("maison invalide".into()))
    }
}

/// Un identifiant de document neuf, comme ceux de Firestore : 20 lettres et
/// chiffres tirés au hasard.
fn new_id() -> Result<String, IglouError> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut id = String::with_capacity(20);
    while id.len() < 20 {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| IglouError::Unexpected("aléatoire".into()))?;
        // 248 = 4 × 62 : pas de lettre plus probable qu'une autre.
        for byte in bytes.into_iter().filter(|b| *b < 248) {
            if id.len() < 20 {
                id.push(char::from(ALPHABET[usize::from(byte % 62)]));
            }
        }
    }
    Ok(id)
}

fn clean(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

async fn read(response: reqwest::Response) -> Result<String, IglouError> {
    let body = ha_tls::read_limited(response, MAX_BYTES)
        .await
        .map_err(|error| match error {
            ha_tls::ReadError::Http(_) => IglouError::Unreachable,
            ha_tls::ReadError::TooLarge => IglouError::Unexpected(error.to_string()),
        })?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn checks_the_settings() {
        assert!(is_project_id("congelateur-ab12c"));
        assert!(!is_project_id("Projet/../x"));
        assert!(is_api_key("AIzaSyA-bcdefghijklmnopqrstu_vwxyz012345"));
        assert!(!is_api_key("cle?x=1"));
        assert_eq!(normalize_code(" ab3 d9z "), Some("AB3D9Z".into()));
        assert_eq!(normalize_code("ABC"), None);
        assert_eq!(normalize_code("AB/D9Z"), None);
    }

    #[test]
    fn new_ids_look_like_firestore_ones() {
        let a = new_id().unwrap();
        let b = new_id().unwrap();
        assert_eq!(a.len(), 20);
        assert!(is_id(&a));
        assert_ne!(a, b);
    }

    #[test]
    fn reads_an_item_uid() {
        assert!(matches!(
            parse_uid("a:abc").unwrap(),
            (Kind::Aliment, "abc")
        ));
        assert!(matches!(parse_uid("l:x_1").unwrap(), (Kind::Libre, "x_1")));
        assert!(parse_uid("z:abc").is_err());
        assert!(parse_uid("l:../x").is_err());
        assert!(parse_uid("abc").is_err());
    }
}
