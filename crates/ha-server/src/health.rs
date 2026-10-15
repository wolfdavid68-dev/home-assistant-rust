//! `ha-server healthcheck` : vérifie qu'un serveur tourne et répond.
//!
//! Docker lance cette commande régulièrement pour savoir si le conteneur
//! est en bonne santé. L'image ne contient ni `curl` ni shell (moins
//! d'outils, c'est moins de prise pour un attaquant) : le programme se
//! vérifie donc lui-même, en demandant la page d'accueil du tableau de bord.

use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TIMEOUT: Duration = Duration::from_secs(3);
/// La première ligne de la réponse suffit (« HTTP/1.1 200 OK »).
const MAX_READ: u64 = 64;

/// Un serveur répond-il déjà à cette adresse ? (Sans rien afficher.)
pub async fn is_running(bind: SocketAddr) -> bool {
    tokio::time::timeout(TIMEOUT, status_line(local_target(bind)))
        .await
        .is_ok_and(|status| status.is_ok_and(|status| is_ok(&status)))
}

pub async fn check(bind: SocketAddr) -> Result<(), Box<dyn Error>> {
    let status = tokio::time::timeout(TIMEOUT, status_line(local_target(bind)))
        .await
        .map_err(|_| "le serveur ne répond pas à temps")??;
    if is_ok(&status) {
        println!("ok");
        Ok(())
    } else {
        Err(format!("réponse inattendue : {status:?}").into())
    }
}

/// Un serveur qui écoute sur « toutes les adresses » (`0.0.0.0`) se joint
/// par la machine elle-même (`127.0.0.1`).
fn local_target(bind: SocketAddr) -> SocketAddr {
    let ip = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind.port())
}

async fn status_line(target: SocketAddr) -> Result<String, Box<dyn Error>> {
    let mut stream = TcpStream::connect(target).await?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    // Concept Rust : `take` limite ce qu'on accepte de lire. Même un faux
    // serveur bavard ne peut pas nous faire remplir la mémoire.
    let mut start = Vec::new();
    stream.take(MAX_READ).read_to_end(&mut start).await?;
    let text = String::from_utf8_lossy(&start);
    Ok(text.lines().next().unwrap_or_default().to_owned())
}

fn is_ok(status: &str) -> bool {
    status.starts_with("HTTP/1.1 200 ") || status.starts_with("HTTP/1.0 200 ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Faux serveur qui répond `response` à la première connexion.
    async fn fake_server(response: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 256];
            let _ = stream.read(&mut request).await;
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn healthy_server_passes() {
        let addr = fake_server("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;
        assert!(check(addr).await.is_ok());
    }

    #[tokio::test]
    async fn error_status_fails() {
        let addr = fake_server("HTTP/1.1 500 Internal Server Error\r\n\r\n").await;
        assert!(check(addr).await.is_err());
    }

    #[tokio::test]
    async fn nothing_listening_fails() {
        // On réserve un port, puis on le libère : plus personne n'y écoute.
        let addr = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(check(addr).await.is_err());
    }

    #[test]
    fn unspecified_address_becomes_localhost() {
        let any: SocketAddr = "0.0.0.0:8123".parse().unwrap();
        assert_eq!(local_target(any), "127.0.0.1:8123".parse().unwrap());
        let any6: SocketAddr = "[::]:8123".parse().unwrap();
        assert_eq!(local_target(any6), "[::1]:8123".parse().unwrap());
        let lan: SocketAddr = "192.168.1.10:8123".parse().unwrap();
        assert_eq!(local_target(lan), lan);
    }

    #[test]
    fn only_200_is_healthy() {
        assert!(is_ok("HTTP/1.1 200 OK"));
        assert!(!is_ok("HTTP/1.1 2000"));
        assert!(!is_ok("HTTP/1.1 401 Unauthorized"));
        assert!(!is_ok(""));
    }
}
