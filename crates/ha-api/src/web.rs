//! Le tableau de bord web : quelques fichiers statiques, intégrés au
//! programme à la compilation.
//!
//! Concept Rust : `include_bytes!` lit un fichier **pendant la compilation**
//! et le met dans le binaire, comme `migrate!` pour les fichiers SQL. Pas de
//! dossier à copier à côté du programme, et personne ne peut modifier la page
//! servie sans recompiler.
//!
//! Deux économies pour les téléphones et les tablettes :
//! - **la compression gzip** : les fichiers texte (JavaScript, CSS, HTML,
//!   SVG) sont compressés une fois, à la première demande, puis gardés en
//!   mémoire. Le tableau de bord passe d'environ 570 Kio à 150 Kio ;
//! - **l'empreinte (`ETag`)** : le navigateur garde sa copie et demande
//!   seulement « a-t-elle changé ? ». Tant que le serveur n'est pas mis à
//!   jour, la réponse est un `304` vide ;
//! - **l'adresse avec l'empreinte** (étape 88) : `index.html` charge
//!   `/app.js?v=9f40f79eaf237fc5`. À cette adresse-là, le fichier ne change
//!   jamais : le navigateur le garde sans même redemander (`immutable`). Une
//!   mise à jour du serveur change l'empreinte, donc l'adresse. Seul
//!   `index.html` est revérifié à chaque ouverture : 1 requête au lieu de 30.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::OnceLock;

use axum::Router;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

/// Politique de sécurité du contenu (CSP) : le navigateur ne charge **que**
/// des ressources venant de ce serveur, et refuse tout script écrit
/// directement dans la page. Même si un nom d'appareil piégé arrivait à
/// s'insérer dans la page, il ne pourrait pas exécuter de code.
///
/// `blob:` dans `img-src` : le dessin de la maison est lu avec le jeton
/// (une balise `<img>` ne sait pas l'envoyer), puis affiché depuis la mémoire
/// de la page. Une image ne peut pas exécuter de script.
///
/// `worker-src 'self'` : le service worker des notifications (`/sw.js`).
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
     img-src 'self' blob:; font-src 'self'; connect-src 'self'; manifest-src 'self'; \
     worker-src 'self'; \
     base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

/// Les fichiers texte : toujours revalidés (avec l'empreinte), pour qu'après
/// une mise à jour du serveur le navigateur prenne la nouvelle version.
const REVALIDATE: &str = "no-cache";
/// Les images et les polices : elles ne changent qu'avec une nouvelle version
/// du serveur, le navigateur peut les garder une semaine.
const ONE_WEEK: &str = "public, max-age=604800";
/// Un fichier demandé à l'adresse qui contient son empreinte : il ne
/// changera jamais à cette adresse.
const FOREVER: &str = "public, max-age=31536000, immutable";

const JS: &str = "text/javascript; charset=utf-8";

/// En dessous de cette taille, la compression ne fait rien gagner.
const MIN_GZIP_SIZE: usize = 1024;

/// Un fichier du tableau de bord.
struct Asset {
    path: &'static str,
    content_type: &'static str,
    cache: &'static str,
    body: &'static [u8],
    /// Calculé à la première demande : l'empreinte et la version compressée.
    prepared: OnceLock<Prepared>,
}

struct Prepared {
    /// Le contenu servi (pour `index.html` : les adresses avec empreinte).
    body: Cow<'static, [u8]>,
    /// L'empreinte courte, pour l'adresse : `/app.js?v=…`.
    version: String,
    etag: HeaderValue,
    /// `None` pour les fichiers déjà compressés (PNG, WOFF2) ou trop petits.
    gzip: Option<(Vec<u8>, HeaderValue)>,
}

macro_rules! asset {
    ($path:literal, $file:literal, $content_type:expr, $cache:expr) => {
        Asset {
            path: $path,
            content_type: $content_type,
            cache: $cache,
            body: include_bytes!(concat!("../web/", $file)),
            prepared: OnceLock::new(),
        }
    };
}

/// Tous les fichiers servis. Le JavaScript est découpé par thème ; l'ordre
/// de chargement est donné par `index.html` (voir l'en-tête de `app.js`).
static ASSETS: [Asset; 32] = [
    asset!("/", "index.html", "text/html; charset=utf-8", REVALIDATE),
    asset!("/utils.js", "utils.js", JS, REVALIDATE),
    asset!("/app.js", "app.js", JS, REVALIDATE),
    asset!("/tiles.js", "tiles.js", JS, REVALIDATE),
    asset!("/dialogs.js", "dialogs.js", JS, REVALIDATE),
    asset!("/editor.js", "editor.js", JS, REVALIDATE),
    asset!("/account.js", "account.js", JS, REVALIDATE),
    asset!("/start.js", "start.js", JS, REVALIDATE),
    asset!("/canvas.js", "canvas.js", JS, REVALIDATE),
    asset!("/house.js", "house.js", JS, REVALIDATE),
    asset!("/agenda.js", "agenda.js", JS, REVALIDATE),
    asset!("/hub.js", "hub.js", JS, REVALIDATE),
    asset!("/sonnette.js", "sonnette.js", JS, REVALIDATE),
    asset!("/colis.js", "colis.js", JS, REVALIDATE),
    asset!("/orbe.js", "orbe.js", JS, REVALIDATE),
    asset!("/cartes.js", "cartes.js", JS, REVALIDATE),
    asset!("/voix.js", "voix.js", JS, REVALIDATE),
    asset!("/pretes.js", "pretes.js", JS, REVALIDATE),
    asset!("/musique.js", "musique.js", JS, REVALIDATE),
    // Le micro (étape 84) : chargé par voix.js dans le fil audio.
    asset!("/ecoute.js", "ecoute.js", JS, REVALIDATE),
    asset!(
        "/style.css",
        "style.css",
        "text/css; charset=utf-8",
        REVALIDATE
    ),
    asset!("/icon.svg", "icon.svg", "image/svg+xml", REVALIDATE),
    // Le dessin de maison par défaut du style « plan d'architecte ».
    asset!("/house.svg", "house.svg", "image/svg+xml", REVALIDATE),
    asset!(
        "/manifest.webmanifest",
        "manifest.webmanifest",
        "application/manifest+json",
        REVALIDATE
    ),
    // Le service worker des notifications.
    asset!("/sw.js", "sw.js", JS, REVALIDATE),
    // L'adresse de retour de l'association SmartThings (étape 86).
    asset!(
        "/smartthings",
        "smartthings.html",
        "text/html; charset=utf-8",
        REVALIDATE
    ),
    // Les icônes de l'appli installée (Android et iPhone ne prennent pas le SVG).
    asset!("/icon-192.png", "icon-192.png", "image/png", ONE_WEEK),
    asset!("/icon-512.png", "icon-512.png", "image/png", ONE_WEEK),
    asset!(
        "/apple-touch-icon.png",
        "apple-touch-icon.png",
        "image/png",
        ONE_WEEK
    ),
    // Polices (licence SIL Open Font License, voir `web/fonts/OFL-*.txt`) :
    // intégrées elles aussi, pour ne rien charger depuis Internet.
    asset!(
        "/fonts/geist.woff2",
        "fonts/geist.woff2",
        "font/woff2",
        ONE_WEEK
    ),
    asset!(
        "/fonts/geist-mono.woff2",
        "fonts/geist-mono.woff2",
        "font/woff2",
        ONE_WEEK
    ),
    asset!(
        "/fonts/bricolage-grotesque.woff2",
        "fonts/bricolage-grotesque.woff2",
        "font/woff2",
        ONE_WEEK
    ),
];

/// Les routes du tableau de bord, une par fichier.
pub(crate) fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    ASSETS.iter().fold(Router::new(), |router, asset| {
        router.route(
            asset.path,
            get(move |uri: Uri, headers: HeaderMap| async move { serve(asset, &uri, &headers) }),
        )
    })
}

impl Asset {
    fn prepared(&self) -> &Prepared {
        self.prepared.get_or_init(|| {
            let body: Cow<'static, [u8]> = if self.path == "/" {
                Cow::Owned(versioned_index(self.body))
            } else {
                Cow::Borrowed(self.body)
            };
            // L'empreinte : le début du SHA-256 du fichier, en hexadécimal.
            let digest = Sha256::digest(&body);
            let mut hex = String::with_capacity(32);
            for byte in &digest[..16] {
                let _ = write!(hex, "{byte:02x}");
            }
            let gzip = compressible(self.content_type)
                .then(|| gzip(&body))
                .flatten()
                // Garder la version compressée seulement si elle est plus petite.
                .filter(|compressed| compressed.len() < body.len())
                .map(|compressed| (compressed, etag(&format!("\"{hex}-gz\""))));
            Prepared {
                version: hex[..16].to_owned(),
                etag: etag(&format!("\"{hex}\"")),
                body,
                gzip,
            }
        })
    }

    /// Chargé par `index.html` à une adresse avec son empreinte ? Pas le
    /// service worker (son adresse doit rester la même), ni `ecoute.js`
    /// (chargé par `voix.js`), ni les fichiers de l'appli installée.
    fn versioned(&self) -> bool {
        (self.content_type == JS && !matches!(self.path, "/sw.js" | "/ecoute.js"))
            || matches!(self.path, "/style.css" | "/icon.svg")
    }
}

/// `index.html`, avec l'empreinte dans chaque adresse : `src="/app.js"` →
/// `src="/app.js?v=9f40f79eaf237fc5"`.
fn versioned_index(index: &'static [u8]) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(index).into_owned();
    for asset in ASSETS.iter().filter(|asset| asset.versioned()) {
        let version = &asset.prepared().version;
        text = text.replace(
            &format!("=\"{}\"", asset.path),
            &format!("=\"{}?v={version}\"", asset.path),
        );
    }
    text.into_bytes()
}

fn etag(value: &str) -> HeaderValue {
    // Que des chiffres hexadécimaux et des guillemets : toujours valide.
    HeaderValue::from_str(value).unwrap_or(HeaderValue::from_static("\"\""))
}

/// Les formats texte gagnent beaucoup à la compression ; PNG et WOFF2 sont
/// déjà compressés.
fn compressible(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type.contains("svg")
        || content_type.contains("json")
}

fn gzip(body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < MIN_GZIP_SIZE {
        return None;
    }
    let mut encoder = GzEncoder::new(Vec::with_capacity(body.len() / 3), Compression::best());
    encoder.write_all(body).ok()?;
    encoder.finish().ok()
}

/// Le navigateur accepte-t-il gzip ? (`Accept-Encoding: gzip, deflate, br`)
/// Une qualité nulle (`gzip;q=0`) veut dire « non ».
fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|item| {
            let mut parts = item.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default();
            let refused = parts.any(|param| {
                param
                    .strip_prefix("q=")
                    .and_then(|q| q.parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            });
            (name.eq_ignore_ascii_case("gzip") || name == "*") && !refused
        })
}

/// La copie du navigateur est-elle encore la bonne ? (`If-None-Match`)
fn still_fresh(headers: &HeaderMap, etag: &HeaderValue) -> bool {
    let Ok(etag) = etag.to_str() else {
        return false;
    };
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        // Un relais peut « affaiblir » l'empreinte : `W/"…"`.
        .map(|tag| tag.strip_prefix("W/").unwrap_or(tag))
        .any(|tag| tag == etag || tag == "*")
}

fn serve(asset: &'static Asset, uri: &Uri, headers: &HeaderMap) -> Response {
    let prepared = asset.prepared();
    let (body, etag, gzipped): (&'static [u8], &HeaderValue, bool) = match &prepared.gzip {
        Some((compressed, gz_etag)) if accepts_gzip(headers) => {
            (compressed.as_slice(), gz_etag, true)
        }
        _ => (prepared.body.as_ref(), &prepared.etag, false),
    };
    // À l'adresse avec la bonne empreinte : gardé pour toujours.
    let forever = asset.versioned()
        && uri
            .query()
            .and_then(|query| query.strip_prefix("v="))
            .is_some_and(|version| version == prepared.version);

    let mut response = if still_fresh(headers, etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut response = body.into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(asset.content_type),
        );
        if gzipped {
            response
                .headers_mut()
                .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        }
        response
    };
    let out = response.headers_mut();
    out.insert(header::ETAG, etag.clone());
    out.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if forever { FOREVER } else { asset.cache }),
    );
    if prepared.gzip.is_some() {
        // Les caches doivent garder une copie par encodage.
        out.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str) -> &'static Asset {
        ASSETS
            .iter()
            .find(|asset| asset.path == path)
            .unwrap_or(&ASSETS[0])
    }

    #[test]
    fn index_loads_files_by_their_fingerprint() {
        let index = String::from_utf8(asset("/").prepared().body.to_vec()).unwrap_or_default();
        let app = &asset("/app.js").prepared().version;
        assert_eq!(app.len(), 16);
        assert!(
            index.contains(&format!("src=\"/app.js?v={app}\"")),
            "{index}"
        );
        assert!(index.contains("href=\"/style.css?v="));
        // Ni le manifeste ni les icônes de l'appli installée.
        assert!(index.contains("href=\"/manifest.webmanifest\""));
        assert!(!index.contains("src=\"/utils.js\""));
    }

    #[test]
    fn a_fingerprinted_address_is_kept_forever() {
        let app = asset("/app.js");
        let version = &app.prepared().version;
        let cache = |uri: &str| {
            serve(app, &uri.parse().unwrap_or_default(), &HeaderMap::new())
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        assert_eq!(cache(&format!("/app.js?v={version}")), FOREVER);
        // Une autre empreinte (une ancienne page) : revérifié.
        assert_eq!(cache("/app.js?v=0000000000000000"), REVALIDATE);
        assert_eq!(cache("/app.js"), REVALIDATE);
        // Le service worker garde son adresse.
        assert!(!asset("/sw.js").versioned());
    }

    fn headers(pairs: &[(header::HeaderName, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn accept_encoding_is_read_like_a_browser() {
        assert!(accepts_gzip(&headers(&[(
            header::ACCEPT_ENCODING,
            "gzip, deflate, br, zstd"
        )])));
        assert!(accepts_gzip(&headers(&[(
            header::ACCEPT_ENCODING,
            "br;q=1.0, gzip;q=0.8"
        )])));
        assert!(accepts_gzip(&headers(&[(header::ACCEPT_ENCODING, "*")])));
        assert!(!accepts_gzip(&headers(&[(
            header::ACCEPT_ENCODING,
            "gzip;q=0"
        )])));
        assert!(!accepts_gzip(&headers(&[(
            header::ACCEPT_ENCODING,
            "br, deflate"
        )])));
        assert!(!accepts_gzip(&headers(&[(
            header::ACCEPT_ENCODING,
            "xgzip"
        )])));
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    #[test]
    fn if_none_match_accepts_lists_and_weak_tags() {
        let etag = HeaderValue::from_static("\"abc\"");
        assert!(still_fresh(
            &headers(&[(header::IF_NONE_MATCH, "\"abc\"")]),
            &etag
        ));
        assert!(still_fresh(
            &headers(&[(header::IF_NONE_MATCH, "\"x\", W/\"abc\"")]),
            &etag
        ));
        assert!(!still_fresh(
            &headers(&[(header::IF_NONE_MATCH, "\"abd\"")]),
            &etag
        ));
        assert!(!still_fresh(&HeaderMap::new(), &etag));
    }

    #[test]
    fn every_path_is_unique_and_text_is_compressed() {
        let mut paths: Vec<_> = ASSETS.iter().map(|asset| asset.path).collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), ASSETS.len());

        let app = ASSETS.iter().find(|asset| asset.path == "/app.js");
        let prepared = app.map(Asset::prepared);
        assert!(prepared.is_some_and(|p| p.gzip.is_some()));
        let font = ASSETS
            .iter()
            .find(|asset| asset.content_type == "font/woff2");
        assert!(font.map(Asset::prepared).is_some_and(|p| p.gzip.is_none()));
    }
}
