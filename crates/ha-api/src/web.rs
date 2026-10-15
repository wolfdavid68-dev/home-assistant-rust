//! Le tableau de bord web : quelques fichiers statiques, intégrés au
//! programme à la compilation.
//!
//! Concept Rust : `include_str!` lit un fichier **pendant la compilation** et
//! le met dans le binaire, comme `migrate!` pour les fichiers SQL. Pas de
//! dossier à copier à côté du programme, et personne ne peut modifier la page
//! servie sans recompiler.

use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const CANVAS_JS: &str = include_str!("../web/canvas.js");
const HOUSE_JS: &str = include_str!("../web/house.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
const ICON_SVG: &str = include_str!("../web/icon.svg");
const HOUSE_SVG: &str = include_str!("../web/house.svg");
const MANIFEST: &str = include_str!("../web/manifest.webmanifest");
// Polices (licence SIL Open Font License, voir `web/fonts/OFL-*.txt`) :
// intégrées elles aussi, pour ne rien charger depuis Internet.
const FONT_GEIST: &[u8] = include_bytes!("../web/fonts/geist.woff2");
const FONT_GEIST_MONO: &[u8] = include_bytes!("../web/fonts/geist-mono.woff2");
const FONT_BRICOLAGE: &[u8] = include_bytes!("../web/fonts/bricolage-grotesque.woff2");

/// Politique de sécurité du contenu (CSP) : le navigateur ne charge **que**
/// des ressources venant de ce serveur, et refuse tout script écrit
/// directement dans la page. Même si un nom d'appareil piégé arrivait à
/// s'insérer dans la page, il ne pourrait pas exécuter de code.
///
/// `blob:` dans `img-src` : le dessin de la maison est lu avec le jeton
/// (une balise `<img>` ne sait pas l'envoyer), puis affiché depuis la mémoire
/// de la page. Une image ne peut pas exécuter de script.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
     img-src 'self' blob:; font-src 'self'; connect-src 'self'; manifest-src 'self'; \
     base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

fn file(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            // Toujours revalider : après une mise à jour du serveur, le
            // navigateur prend la nouvelle version.
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
}

pub(crate) async fn index() -> impl IntoResponse {
    file("text/html; charset=utf-8", INDEX_HTML)
}

pub(crate) async fn app_js() -> impl IntoResponse {
    file("text/javascript; charset=utf-8", APP_JS)
}

pub(crate) async fn canvas_js() -> impl IntoResponse {
    file("text/javascript; charset=utf-8", CANVAS_JS)
}

pub(crate) async fn house_js() -> impl IntoResponse {
    file("text/javascript; charset=utf-8", HOUSE_JS)
}

pub(crate) async fn style_css() -> impl IntoResponse {
    file("text/css; charset=utf-8", STYLE_CSS)
}

pub(crate) async fn icon_svg() -> impl IntoResponse {
    file("image/svg+xml", ICON_SVG)
}

/// Le dessin de maison par défaut du style « plan d'architecte ».
pub(crate) async fn house_svg() -> impl IntoResponse {
    file("image/svg+xml", HOUSE_SVG)
}

pub(crate) async fn manifest() -> impl IntoResponse {
    file("application/manifest+json", MANIFEST)
}

/// Une police : elle ne change qu'avec une nouvelle version du serveur, le
/// navigateur peut la garder une semaine.
fn font(body: &'static [u8]) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("font/woff2")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=604800"),
            ),
        ],
        body,
    )
}

pub(crate) async fn font_geist() -> impl IntoResponse {
    font(FONT_GEIST)
}

pub(crate) async fn font_geist_mono() -> impl IntoResponse {
    font(FONT_GEIST_MONO)
}

pub(crate) async fn font_bricolage() -> impl IntoResponse {
    font(FONT_BRICOLAGE)
}
