//! La découverte : quand on dit « Alexa, découvre mes appareils », l'Echo
//! envoie sur le réseau local un message UDP (SSDP, port 1900, adresse de
//! groupe 239.255.255.250) qui demande « y a-t-il des ponts ? ». On répond
//! en donnant l'adresse de notre fiche `description.xml`.
//!
//! Sécurité : on ne répond qu'aux adresses autorisées, et pas plus de
//! [`MAX_REPLIES`] fois par fenêtre de [`WINDOW`]. Une réponse est plus grosse
//! que la question : sans limite, quelqu'un pourrait se servir de nous pour
//! inonder une autre machine en usurpant son adresse.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

use crate::server::{Allowed, Identity};

/// Adresse de groupe SSDP.
pub const MULTICAST: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
/// Port SSDP standard.
pub const PORT: u16 = 1900;
const MAX_REPLIES: usize = 30;
const WINDOW: Duration = Duration::from_secs(10);
/// Taille maximale d'une question (un vrai M-SEARCH fait ~150 octets).
const MAX_REQUEST: usize = 2048;

/// Les recherches auxquelles un pont Hue répond.
const SEARCH_TARGETS: [&str; 3] = [
    "ssdp:all",
    "upnp:rootdevice",
    "urn:schemas-upnp-org:device:basic:1",
];

/// La réponse à une question SSDP, ou `None` si elle ne nous concerne pas.
#[must_use]
pub fn reply(request: &str, identity: &Identity) -> Option<String> {
    let mut lines = request.lines();
    if !lines
        .next()?
        .trim()
        .eq_ignore_ascii_case("M-SEARCH * HTTP/1.1")
    {
        return None;
    }
    let mut man = None;
    let mut st = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match name.trim().to_ascii_lowercase().as_str() {
            "man" => man = Some(value.to_owned()),
            "st" => st = Some(value.to_ascii_lowercase()),
            _ => {}
        }
    }
    if man.as_deref() != Some("ssdp:discover") {
        return None;
    }
    let st = st.filter(|st| SEARCH_TARGETS.contains(&st.as_str()))?;
    // À « tout le monde » (ssdp:all), on répond en tant que pont.
    let st = if st == "ssdp:all" {
        "urn:schemas-upnp-org:device:basic:1".to_owned()
    } else {
        st
    };
    Some(format!(
        "HTTP/1.1 200 OK\r\n\
         CACHE-CONTROL: max-age=60\r\n\
         EXT:\r\n\
         LOCATION: {location}\r\n\
         SERVER: FreeRTOS/6.0.5, UPnP/1.0, IpBridge/1.17.0\r\n\
         hue-bridgeid: {bridge_id}\r\n\
         ST: {st}\r\n\
         USN: uuid:{uuid}::upnp:rootdevice\r\n\
         \r\n",
        location = identity.location(),
        bridge_id = identity.bridge_id(),
        uuid = identity.uuid(),
    ))
}

/// Ouvre la socket UDP de découverte : port `port` sur toutes les
/// interfaces, abonnée au groupe SSDP sur l'interface `interface`.
///
/// Le port 1900 est souvent déjà utilisé (le service « Découverte SSDP » de
/// Windows, un autre logiciel…) : on demande à le **partager**
/// (`SO_REUSEADDR`), comme le font tous les logiciels UPnP.
pub fn bind(interface: Ipv4Addr, port: u16) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).into())?;
    if let Err(error) = socket.join_multicast_v4(&MULTICAST, &interface) {
        // Sans le groupe, les recherches envoyées directement à notre
        // adresse marchent encore : on prévient, sans s'arrêter.
        tracing::warn!("Alexa : abonnement au groupe SSDP impossible sur {interface} ({error})");
    }
    socket.set_nonblocking(true)?;
    UdpSocket::from_std(socket.into())
}

/// Répond aux recherches jusqu'à l'arrêt.
pub async fn serve(socket: UdpSocket, identity: Identity, allowed: Arc<Allowed>) {
    let mut buffer = vec![0u8; MAX_REQUEST];
    let mut window_start = Instant::now();
    let mut replies = 0usize;
    loop {
        let (length, from) = match socket.recv_from(&mut buffer).await {
            Ok(received) => received,
            Err(error) => {
                tracing::debug!("SSDP : réception impossible ({error})");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        if !allowed.permits(from.ip()) {
            continue;
        }
        let Some(request) = buffer
            .get(..length)
            .and_then(|b| std::str::from_utf8(b).ok())
        else {
            continue;
        };
        let Some(answer) = reply(request, &identity) else {
            continue;
        };
        if window_start.elapsed() > WINDOW {
            window_start = Instant::now();
            replies = 0;
        }
        if replies >= MAX_REPLIES {
            continue;
        }
        replies += 1;
        tracing::debug!("SSDP : recherche de {from}, on répond");
        if let Err(error) = socket.send_to(answer.as_bytes(), from).await {
            tracing::debug!("SSDP : réponse impossible à {from} ({error})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity::new(Ipv4Addr::new(192, 168, 1, 20), 80)
    }

    const ECHO_SEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\n\
        MAN: \"ssdp:discover\"\r\nMX: 3\r\nST: urn:schemas-upnp-org:device:basic:1\r\n\r\n";

    #[test]
    fn echo_searches_get_the_bridge_address() {
        let answer = reply(ECHO_SEARCH, &identity()).unwrap();
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(answer.contains("LOCATION: http://192.168.1.20:80/description.xml\r\n"));
        assert!(answer.contains("ST: urn:schemas-upnp-org:device:basic:1\r\n"));
        assert!(answer.contains("hue-bridgeid: 001788FFFE"));
        assert!(answer.ends_with("\r\n\r\n"));

        let all = ECHO_SEARCH.replace("urn:schemas-upnp-org:device:basic:1", "ssdp:all");
        assert!(
            reply(&all, &identity())
                .unwrap()
                .contains("ST: urn:schemas-upnp-org:device:basic:1")
        );
        // En-têtes en minuscules : accepté aussi.
        assert!(reply(&ECHO_SEARCH.to_lowercase(), &identity()).is_some());
    }

    #[test]
    fn other_messages_are_ignored() {
        for message in [
            ECHO_SEARCH.replace(
                "urn:schemas-upnp-org:device:basic:1",
                "urn:dial-multiscreen-org:service:dial:1",
            ),
            ECHO_SEARCH.replace("ssdp:discover", "ssdp:alive"),
            ECHO_SEARCH.replace("M-SEARCH", "NOTIFY"),
            String::new(),
            "n'importe quoi".to_owned(),
        ] {
            assert_eq!(reply(&message, &identity()), None, "{message}");
        }
    }

    #[tokio::test]
    async fn the_socket_answers_a_search() {
        // Port choisi par le système : le test ne gêne pas un vrai SSDP.
        let socket = bind(Ipv4Addr::LOCALHOST, 0).unwrap();
        let port = socket.local_addr().unwrap().port();
        let task = tokio::spawn(serve(socket, identity(), Arc::new(Allowed::LocalNetwork)));

        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client
            .send_to(ECHO_SEARCH.as_bytes(), ("127.0.0.1", port))
            .await
            .unwrap();
        let mut buffer = [0u8; 1024];
        let (length, _) =
            tokio::time::timeout(Duration::from_secs(3), client.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        let answer = std::str::from_utf8(&buffer[..length]).unwrap();
        assert!(answer.contains("description.xml"), "{answer}");
        task.abort();
    }
}
