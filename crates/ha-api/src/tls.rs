//! Le tableau de bord en HTTPS sur le réseau de la maison (étape 79).
//!
//! Un seul port (8123) pour tout :
//! - une connexion **TLS** va au tableau de bord et à l'API, comme avant ;
//! - une requête **HTTP en clair** (un ancien favori, `http://…:8123`) reçoit
//!   une redirection vers `https://` : aucun mot de passe ne circule plus en
//!   clair sur le Wi-Fi. Une seule exception : `/maison-ca.crt`, le
//!   certificat de l'autorité de la maison, à installer sur les appareils
//!   (il est public, et il faut pouvoir l'obtenir avant de lui faire
//!   confiance).
//!
//! Les poignées de main TLS se font chacune dans leur tâche, avec un délai
//! maximal et un nombre limité en même temps : un client lent ou malveillant
//! ne bloque pas les autres.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::serve::{Listener, ListenerExt as _};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use crate::{ApiState, CancellationToken};

/// Le chemin du certificat de l'autorité, servi en clair.
pub const CA_PATH: &str = "/maison-ca.crt";

/// Délai pour la poignée de main TLS (ou la requête en clair).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Poignées de main en cours au plus : au-delà, les connexions sont fermées.
const MAX_HANDSHAKES: usize = 128;
/// Taille maximale de l'en-tête d'une requête en clair.
const MAX_PLAIN_HEAD: usize = 8 * 1024;

/// Ce qu'il faut pour servir en HTTPS.
#[derive(Clone)]
pub struct Https {
    /// La configuration TLS (le certificat du serveur, renouvelable).
    pub config: Arc<rustls::ServerConfig>,
    /// Le certificat de l'autorité de la maison (DER), servi en clair.
    pub ca_der: Arc<[u8]>,
}

/// Comme [`crate::serve`], mais en HTTPS.
pub async fn serve_https(
    listener: TcpListener,
    state: Arc<ApiState>,
    https: Https,
) -> io::Result<()> {
    let shutdown = state.shutdown_signal();
    let local = listener.local_addr()?;
    let (sender, receiver) = mpsc::channel(64);
    tokio::spawn(accept_loop(listener, https, sender, shutdown.clone()));
    let app = crate::router(state).into_make_service_with_connect_info::<SocketAddr>();
    // `tap_io` : axum sait alors donner l'adresse du client (`ConnectInfo`)
    // pour notre propre `Listener`.
    let listener = TlsListener { receiver, local }.tap_io(|_| {});
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
}

/// Accepte les connexions TCP et fait la poignée de main de chacune à part.
async fn accept_loop(
    listener: TcpListener,
    https: Https,
    sender: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
    shutdown: CancellationToken,
) {
    let acceptor = TlsAcceptor::from(Arc::clone(&https.config));
    let limit = Arc::new(Semaphore::new(MAX_HANDSHAKES));
    loop {
        let accepted = tokio::select! {
            () = shutdown.cancelled() => return,
            accepted = listener.accept() => accepted,
        };
        let (tcp, client) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                // Trop de fichiers ouverts, par exemple : on souffle un peu.
                tracing::warn!("HTTPS : connexion refusée ({error})");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
            continue; // trop de poignées de main en cours : on ferme
        };
        let acceptor = acceptor.clone();
        let sender = sender.clone();
        let ca = Arc::clone(&https.ca_der);
        tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(HANDSHAKE_TIMEOUT, async move {
                let mut first = [0u8; 1];
                match tcp.peek(&mut first).await {
                    // 0x16 : le premier octet d'une poignée de main TLS.
                    Ok(1) if first[0] == 0x16 => {
                        if let Ok(stream) = acceptor.accept(tcp).await {
                            let _ = sender.send((stream, client)).await;
                        }
                    }
                    Ok(1) => answer_plain(tcp, &ca).await,
                    _ => {}
                }
            })
            .await;
        });
    }
}

/// Les connexions TLS prêtes, pour axum.
struct TlsListener {
    receiver: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
}

impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.receiver.recv().await {
            Some(connection) => connection,
            // Plus de connexions (arrêt) : axum s'arrête de lui-même.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local)
    }
}

/// Répond à une requête HTTP en clair, puis ferme.
async fn answer_plain(mut tcp: TcpStream, ca: &[u8]) {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_PLAIN_HEAD {
        match tcp.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    }
    let _ = tcp.write_all(&plain_response(&head, ca)).await;
    let _ = tcp.shutdown().await;
}

/// La réponse à une requête en clair : le certificat de l'autorité, ou une
/// redirection vers la même adresse en `https://`.
pub(crate) fn plain_response(head: &[u8], ca: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let mut request = lines.next().unwrap_or_default().split(' ');
    let method = request.next().unwrap_or_default();
    let target = request.next().unwrap_or_default();
    let host = lines
        .take_while(|line| !line.is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("host")
                .then(|| value.trim())
        })
        .filter(|host| is_valid_host(host));

    if matches!(method, "GET" | "HEAD") && target == CA_PATH {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/x-x509-ca-cert\r\n\
             Content-Disposition: attachment; filename=\"maison-ca.crt\"\r\n\
             Content-Length: {}\r\n\
             Cache-Control: no-store\r\n\
             X-Content-Type-Options: nosniff\r\n\
             Connection: close\r\n\r\n",
            ca.len()
        )
        .into_bytes();
        if method == "GET" {
            response.extend_from_slice(ca);
        }
        return response;
    }
    let Some(host) = host else {
        let body = "Utilise https:// pour ouvrir le tableau de bord.\n";
        return format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes();
    };
    // Le chemin est recopié seulement s'il est sage (pas de retour à la
    // ligne, qui permettrait d'ajouter un en-tête à la réponse).
    let path = if target.starts_with('/')
        && target.len() <= 2048
        && target.bytes().all(|b| (0x21..0x7f).contains(&b))
    {
        target
    } else {
        "/"
    };
    let location = format!("https://{host}{path}");
    let body = format!("Le tableau de bord est en HTTPS : {location}\n");
    format!(
        "HTTP/1.1 301 Moved Permanently\r\nLocation: {location}\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Un en-tête `Host` raisonnable : un nom ou une adresse, avec un port.
fn is_valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(head: &str) -> String {
        String::from_utf8(plain_response(head.as_bytes(), b"CA")).unwrap_or_default()
    }

    #[test]
    fn plain_http_is_sent_to_https_with_the_same_address() {
        let answer = response("GET /api/states?x=1 HTTP/1.1\r\nHost: 192.168.1.20:8123\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 301 "), "{answer}");
        assert!(answer.contains("\r\nLocation: https://192.168.1.20:8123/api/states?x=1\r\n"));
        // Le nom d'en-tête n'est pas sensible à la casse.
        let answer = response("GET / HTTP/1.1\r\nhOsT: maison.local:8123\r\n\r\n");
        assert!(
            answer.contains("Location: https://maison.local:8123/\r\n"),
            "{answer}"
        );
    }

    #[test]
    fn the_ca_certificate_is_served_in_plain_http() {
        let answer = response("GET /maison-ca.crt HTTP/1.1\r\nHost: 192.168.1.20:8123\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
        assert!(answer.contains("application/x-x509-ca-cert"));
        assert!(answer.ends_with("\r\n\r\nCA"));
        let head = response("HEAD /maison-ca.crt HTTP/1.1\r\n\r\n");
        assert!(head.ends_with("\r\n\r\n"), "{head}");
    }

    #[test]
    fn nothing_can_be_injected_in_the_redirection() {
        // Un chemin ou un hôte piégé ne passe pas dans la réponse.
        let answer = response("GET /\u{1b}x HTTP/1.1\r\nHost: a.local\r\n\r\n");
        assert!(
            answer.contains("Location: https://a.local/\r\n"),
            "{answer}"
        );
        let answer = response("GET / HTTP/1.1\r\nHost: evil\"<script>\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 400 "), "{answer}");
        let answer = response("GET / HTTP/1.1\r\n\r\n");
        assert!(answer.starts_with("HTTP/1.1 400 "), "{answer}");
    }
}
