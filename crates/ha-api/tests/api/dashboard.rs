//! Les fichiers du tableau de bord, leurs en-têtes et le HTTPS.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use crate::common::TestApp;

#[tokio::test]
async fn dashboard_files_are_served_with_security_headers() {
    let app = TestApp::new().await;
    for (uri, content_type) in [
        ("/", "text/html; charset=utf-8"),
        ("/utils.js", "text/javascript; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/tiles.js", "text/javascript; charset=utf-8"),
        ("/dialogs.js", "text/javascript; charset=utf-8"),
        ("/editor.js", "text/javascript; charset=utf-8"),
        ("/account.js", "text/javascript; charset=utf-8"),
        ("/start.js", "text/javascript; charset=utf-8"),
        ("/canvas.js", "text/javascript; charset=utf-8"),
        ("/house.js", "text/javascript; charset=utf-8"),
        ("/agenda.js", "text/javascript; charset=utf-8"),
        ("/hub.js", "text/javascript; charset=utf-8"),
        ("/sonnette.js", "text/javascript; charset=utf-8"),
        ("/colis.js", "text/javascript; charset=utf-8"),
        ("/voix.js", "text/javascript; charset=utf-8"),
        ("/pretes.js", "text/javascript; charset=utf-8"),
        ("/musique.js", "text/javascript; charset=utf-8"),
        ("/ecoute.js", "text/javascript; charset=utf-8"),
        ("/house.svg", "image/svg+xml"),
        ("/style.css", "text/css; charset=utf-8"),
        ("/icon.svg", "image/svg+xml"),
        ("/manifest.webmanifest", "application/manifest+json"),
        ("/fonts/geist.woff2", "font/woff2"),
        ("/fonts/geist-mono.woff2", "font/woff2"),
        ("/fonts/bricolage-grotesque.woff2", "font/woff2"),
        ("/sw.js", "text/javascript; charset=utf-8"),
        ("/smartthings", "text/html; charset=utf-8"),
        ("/icon-192.png", "image/png"),
        ("/icon-512.png", "image/png"),
        ("/apple-touch-icon.png", "image/png"),
    ] {
        let response = app
            .router()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_TYPE], content_type, "{uri}");
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("script-src 'self'"),
            "{uri}"
        );
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
        // Le service worker des notifications, et rien d'autre.
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("worker-src 'self';"),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn embedded_fonts_are_real_woff2_and_allowed_by_the_policy() {
    let app = TestApp::new().await;
    let response = app
        .router()
        .oneshot(
            Request::builder()
                .uri("/fonts/geist.woff2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let policy = response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_owned();
    // Les polices viennent du serveur lui-même, jamais d'un site extérieur.
    assert!(policy.contains("font-src 'self'"), "{policy}");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.starts_with(b"wOF2"), "pas un fichier WOFF2");
}

#[tokio::test]
async fn api_errors_also_carry_security_headers() {
    let app = TestApp::new().await;
    let response = app
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/states")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY)
    );
    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
}

#[test]
fn dashboard_never_uses_inner_html() {
    // Règle de sécurité : les noms d'appareils viennent du réseau. La page ne
    // doit jamais les insérer comme du HTML.
    let scripts = [
        ("utils.js", include_str!("../../web/utils.js")),
        ("app.js", include_str!("../../web/app.js")),
        ("tiles.js", include_str!("../../web/tiles.js")),
        ("dialogs.js", include_str!("../../web/dialogs.js")),
        ("editor.js", include_str!("../../web/editor.js")),
        ("account.js", include_str!("../../web/account.js")),
        ("start.js", include_str!("../../web/start.js")),
        ("canvas.js", include_str!("../../web/canvas.js")),
        ("house.js", include_str!("../../web/house.js")),
        ("agenda.js", include_str!("../../web/agenda.js")),
        ("hub.js", include_str!("../../web/hub.js")),
        ("sonnette.js", include_str!("../../web/sonnette.js")),
        ("colis.js", include_str!("../../web/colis.js")),
        ("orbe.js", include_str!("../../web/orbe.js")),
        ("cartes.js", include_str!("../../web/cartes.js")),
        ("voix.js", include_str!("../../web/voix.js")),
        ("pretes.js", include_str!("../../web/pretes.js")),
        ("musique.js", include_str!("../../web/musique.js")),
        ("ecoute.js", include_str!("../../web/ecoute.js")),
        ("sw.js", include_str!("../../web/sw.js")),
    ];
    for (name, script) in scripts {
        dashboard_script_is_safe(name, script);
    }
    // Chaque script chargé par la page est vérifié ci-dessus : un nouveau
    // fichier ne peut pas être oublié.
    let index = include_str!("../../web/index.html");
    for src in index.split("<script src=\"/").skip(1) {
        let name = src.split('"').next().unwrap();
        assert!(
            scripts.iter().any(|(checked, _)| *checked == name),
            "{name} chargé par index.html mais pas vérifié"
        );
    }
}

/// Une demande du tableau de bord, avec des en-têtes choisis.
async fn get_dashboard_file(
    app: &TestApp,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
) -> axum::response::Response {
    let mut request = Request::builder().uri(uri);
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    app.router()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn dashboard_text_files_are_gzipped_for_browsers_that_accept_it() {
    use std::io::Read as _;

    let app = TestApp::new().await;
    let gzip = [(header::ACCEPT_ENCODING, "gzip, deflate, br")];
    let original = include_bytes!("../../web/app.js");

    let response = get_dashboard_file(&app, "/app.js", &gzip).await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
    assert_eq!(headers[header::VARY], "Accept-Encoding");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    // Les en-têtes de sécurité restent là.
    assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY));
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.len() * 2 < original.len(), "{} octets", body.len());
    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_end(&mut unpacked)
        .unwrap();
    assert_eq!(unpacked, original);

    // Sans gzip annoncé (ou refusé), le fichier part tel quel.
    for headers in [&[][..], &[(header::ACCEPT_ENCODING, "gzip;q=0")][..]] {
        let response = get_dashboard_file(&app, "/app.js", headers).await;
        assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
        assert_eq!(response.headers()[header::VARY], "Accept-Encoding");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], original);
    }

    // Les polices et les images sont déjà compressées : jamais deux fois.
    for uri in ["/fonts/geist.woff2", "/icon-192.png"] {
        let response = get_dashboard_file(&app, uri, &gzip).await;
        assert!(
            !response.headers().contains_key(header::CONTENT_ENCODING),
            "{uri}"
        );
        assert!(!response.headers().contains_key(header::VARY), "{uri}");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=604800",
            "{uri}"
        );
    }
}

#[tokio::test]
async fn dashboard_files_are_not_sent_again_while_unchanged() {
    let app = TestApp::new().await;
    for accept in ["gzip", "identity"] {
        let response =
            get_dashboard_file(&app, "/style.css", &[(header::ACCEPT_ENCODING, accept)]).await;
        let etag = response.headers()[header::ETAG]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");

        // Le navigateur redemande avec son empreinte : rien à renvoyer.
        let again = get_dashboard_file(
            &app,
            "/style.css",
            &[
                (header::ACCEPT_ENCODING, accept),
                (header::IF_NONE_MATCH, &etag),
            ],
        )
        .await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{accept}");
        assert_eq!(again.headers()[header::ETAG], etag.as_str());
        assert!(
            again
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        let body = axum::body::to_bytes(again.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.is_empty());

        // Une autre empreinte (ancienne version) : le fichier est renvoyé.
        let old = get_dashboard_file(
            &app,
            "/style.css",
            &[
                (header::ACCEPT_ENCODING, accept),
                (header::IF_NONE_MATCH, "\"ancienne\""),
            ],
        )
        .await;
        assert_eq!(old.status(), StatusCode::OK, "{accept}");
    }

    // La version compressée et la version brute n'ont pas la même empreinte.
    let gz = get_dashboard_file(&app, "/style.css", &[(header::ACCEPT_ENCODING, "gzip")]).await;
    let raw = get_dashboard_file(&app, "/style.css", &[]).await;
    assert_ne!(gz.headers()[header::ETAG], raw.headers()[header::ETAG]);
}

fn dashboard_script_is_safe(name: &str, script: &str) {
    for forbidden in [
        ".innerHTML",
        ".outerHTML",
        ".insertAdjacentHTML(",
        "document.write(",
        "eval(",
        "new Function(",
    ] {
        assert!(
            !script.contains(forbidden),
            "{forbidden} interdit dans {name}"
        );
    }
}

/// Envoie une requête HTTP/1.1 brute et renvoie toute la réponse.
async fn raw_request<S>(mut stream: S, request: &str) -> Vec<u8>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response).await;
    response
}

#[tokio::test]
async fn the_dashboard_is_served_in_https_and_plain_http_is_redirected() {
    let app = TestApp::new().await;
    let token = app.login().await;
    let ca = ha_tls::local_ca::LocalCa::generate().unwrap();
    let names = [ha_tls::local_ca::LocalName::parse("127.0.0.1").unwrap()];
    let cert = ha_tls::local_ca::ServerCert::new(ca.issue_server(&names).unwrap());
    let https = ha_api::Https {
        config: ha_tls::local_ca::server_config(cert).unwrap(),
        ca_der: Arc::from(ca.der()),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ha_api::serve_https(listener, Arc::clone(&app.state), https));

    // Un client qui ne fait confiance qu'à l'autorité de la maison.
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(ca.der().to_vec()))
        .unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let server = rustls::pki_types::ServerName::IpAddress(addr.ip().into());
    let tls = connector.connect(server, tcp).await.unwrap();
    let response = raw_request(
        tls,
        &format!(
            "GET /api/ HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\n\
             Connection: close\r\n\r\n"
        ),
    )
    .await;
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 200 "), "{text}");
    assert!(text.contains("API en marche"), "{text}");
    // Les en-têtes de sécurité sont là aussi en HTTPS.
    assert!(
        text.to_ascii_lowercase()
            .contains("content-security-policy")
    );

    // En clair : redirigé vers la même adresse en https://.
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let response = raw_request(
        tcp,
        &format!("GET /api/states HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"),
    )
    .await;
    let text = String::from_utf8_lossy(&response);
    assert!(text.starts_with("HTTP/1.1 301 "), "{text}");
    assert!(
        text.contains(&format!("\r\nLocation: https://{addr}/api/states\r\n")),
        "{text}"
    );

    // Sauf le certificat de l'autorité, à installer sur les appareils.
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let response = raw_request(
        tcp,
        &format!("GET /maison-ca.crt HTTP/1.1\r\nHost: {addr}\r\n\r\n"),
    )
    .await;
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(response.ends_with(ca.der()));

    // Une autre autorité (un appareil qui ne connaît pas la maison) : refusé.
    let stranger = ha_tls::local_ca::LocalCa::generate().unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(
            stranger.der().to_vec(),
        ))
        .unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let refused = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(
            rustls::pki_types::ServerName::IpAddress(addr.ip().into()),
            tcp,
        )
        .await;
    assert!(refused.is_err());
}
