//! Trouver les enceintes sans rien régler : on demande sur le réseau local
//! (SSDP, comme l'appli Sonos) « y a-t-il des enceintes Sonos ? », et
//! chacune répond avec l'adresse de sa fiche.
//!
//! Il suffit d'en trouver une : elle connaît toutes les autres (voir
//! [`crate::topology`]). Dans un conteneur Docker sans le réseau de la
//! machine, la question ne sort pas : il faut alors donner l'adresse d'une
//! enceinte (`HA_SONOS=192.168.1.30`).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::topology::host_of;

const MULTICAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\n\
                      HOST: 239.255.255.250:1900\r\n\
                      MAN: \"ssdp:discover\"\r\n\
                      MX: 1\r\n\
                      ST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n\r\n";

/// Les adresses des enceintes qui ont répondu pendant `wait`.
pub async fn search(wait: Duration) -> Vec<String> {
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
        return Vec::new();
    };
    // Deux fois : un paquet UDP peut se perdre.
    for _ in 0..2 {
        if let Err(error) = socket.send_to(SEARCH.as_bytes(), MULTICAST).await {
            tracing::debug!("Sonos : recherche impossible ({error})");
            return Vec::new();
        }
    }
    let mut hosts = Vec::new();
    let mut buffer = vec![0u8; 2048];
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Ok((length, _))) =
        tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)).await
    {
        let reply = String::from_utf8_lossy(&buffer[..length]);
        if let Some(host) = location(&reply).and_then(|l| host_of(&l))
            && !hosts.contains(&host)
            && hosts.len() < 64
        {
            hosts.push(host);
        }
    }
    hosts
}

/// La ligne `LOCATION:` d'une réponse SSDP d'enceinte Sonos.
fn location(reply: &str) -> Option<String> {
    let mut location = None;
    let mut sonos = false;
    for line in reply.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "location" => location = Some(value.trim().to_owned()),
            "st" | "server" if value.contains("ZonePlayer") || value.contains("Sonos") => {
                sonos = true;
            }
            _ => {}
        }
    }
    location.filter(|_| sonos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_location_of_a_sonos() {
        let reply = "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age = 1800\r\nEXT:\r\n\
                     LOCATION: http://192.168.1.30:1400/xml/device_description.xml\r\n\
                     SERVER: Linux UPnP/1.0 Sonos/80.1-55014 (ZPS27)\r\n\
                     ST: urn:schemas-upnp-org:device:ZonePlayer:1\r\n\r\n";
        assert_eq!(
            location(reply).and_then(|l| host_of(&l)).as_deref(),
            Some("192.168.1.30:1400")
        );
        // Une box ou une imprimante qui répond quand même : ignorée.
        let other = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.1:1400/desc.xml\r\nST: upnp:rootdevice\r\n\r\n";
        assert_eq!(location(other), None);
    }
}
