//! Un faux compteur Linky derrière un module réseau (ser2net).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "dans un test, un échec doit simplement faire échouer le test"
)]

use std::sync::Arc;
use std::time::Duration;

use ha_core::Core;
use ha_teleinfo::frame::checksum;
use ha_teleinfo::{Config, Teleinfo};
use tokio::io::AsyncWriteExt;

/// Une trame historique, avec le bit de parité sur chaque octet (comme un
/// pont série qui ne la retire pas).
fn frame(groups: &[(&str, &str)]) -> Vec<u8> {
    let mut bytes = vec![0x02];
    for (label, value) in groups {
        let data = format!("{label} {value}");
        bytes.push(0x0A);
        bytes.extend_from_slice(data.as_bytes());
        bytes.push(b' ');
        bytes.push(checksum(data.as_bytes()));
        bytes.push(0x0D);
    }
    bytes.push(0x03);
    bytes
        .into_iter()
        .map(|b| if b.count_ones() % 2 == 1 { b | 0x80 } else { b })
        .collect()
}

#[tokio::test]
async fn a_meter_behind_a_network_bridge() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // On se branche au milieu d'une trame : la fin est ignorée.
        socket.write_all(b"P 00920 X\r\x03").await.unwrap();
        let groups = [
            ("ADCO", "031762120246"),
            ("OPTARIF", "HC.."),
            ("ISOUSC", "30"),
            ("HCHC", "012345678"),
            ("HCHP", "023456789"),
            ("PTEC", "HC.."),
            ("IINST", "004"),
            ("PAPP", "00920"),
        ];
        let bytes = frame(&groups);
        // Envoyée en petits morceaux, comme sur une vraie liaison.
        for chunk in bytes.chunks(9) {
            socket.write_all(chunk).await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let core = Arc::new(Core::new());
    let config = Config::parse(&format!("tcp://{address}"), None).unwrap();
    let teleinfo = Teleinfo::start(config, Arc::clone(&core));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let state = |id: &str| core.states.get(&id.parse().unwrap());
    let total = state("sensor.linky_energie").expect("consommation totale");
    assert_eq!(total.state(), "35802.467");
    assert_eq!(total.attributes()["unit_of_measurement"], "kWh");
    assert_eq!(total.attributes()["state_class"], "total_increasing");
    assert_eq!(total.attributes()["compteur"], "…0246");
    assert_eq!(total.attributes()["option_tarifaire"], "Heures creuses");
    assert_eq!(
        state("sensor.linky_heures_creuses").unwrap().state(),
        "12345.678"
    );
    assert_eq!(
        state("sensor.linky_heures_pleines").unwrap().attributes()["friendly_name"],
        "Linky heures pleines"
    );
    assert_eq!(state("sensor.linky_puissance").unwrap().state(), "920");
    assert_eq!(state("sensor.linky_intensite").unwrap().state(), "4");
    assert_eq!(
        state("sensor.linky_periode").unwrap().state(),
        "Heures creuses"
    );
    // Pas de tension en mode historique, ni de production.
    assert!(state("sensor.linky_tension").is_none());
    assert!(state("sensor.linky_production").is_none());
    teleinfo.stop().await;
}
