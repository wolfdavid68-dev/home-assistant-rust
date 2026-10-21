//! `HA_TLS=on` : le tableau de bord en HTTPS sur le réseau de la maison
//! (étape 79). L'autorité de la maison et le certificat du serveur viennent
//! de [`ha_tls::local_ca`] ; ici, on choisit les noms, on prépare le serveur
//! et on renouvelle le certificat chaque semaine.

use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ha_tls::local_ca::{self, LocalCa, LocalName, ServerCert};
use tracing::{info, warn};

/// Le certificat du serveur est refait chaque semaine (il vaut 397 jours) :
/// il suit aussi un changement d'adresse de la machine.
const RENEW_EVERY: Duration = Duration::from_hours(7 * 24);

/// `HA_TLS` : `on` pour le HTTPS, `off` (par défaut) pour le HTTP.
pub fn enabled() -> Result<bool, String> {
    match std::env::var("HA_TLS").as_deref() {
        Err(_) | Ok("off" | "") => Ok(false),
        Ok("on") => Ok(true),
        Ok(other) => Err(format!("HA_TLS : « {other} » invalide (on ou off)")),
    }
}

/// Le dossier de l'autorité : `HA_TLS_DIR`, sinon celui de la clé qui
/// chiffre la base (à part des sauvegardes, comme elle).
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("HA_TLS_DIR") {
        return dir.into();
    }
    let key = std::env::var_os("HA_KEY_PATH").map_or_else(
        || PathBuf::from(crate::db_path()).with_extension("key"),
        PathBuf::from,
    );
    key.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Les noms du certificat : `localhost`, l'adresse de la machine sur le
/// réseau, son nom en `.local`, et ceux de `HA_TLS_NOMS` (séparés par des
/// virgules).
pub fn names(bind: SocketAddr) -> Result<Vec<LocalName>, String> {
    let mut names = vec![
        LocalName::Dns("localhost".to_owned()),
        LocalName::Ip(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        LocalName::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ];
    let mut add = |name: LocalName| {
        if !names.contains(&name) {
            names.push(name);
        }
    };
    if !bind.ip().is_unspecified()
        && let Ok(name) = LocalName::parse(&bind.ip().to_string())
    {
        add(name);
    }
    if let Some(ip) = lan_address()
        && let Ok(name) = LocalName::parse(&ip.to_string())
    {
        add(name);
    }
    if let Some(host) = host_name()
        && let Ok(name) = LocalName::parse(&format!("{host}.local"))
    {
        add(name);
    }
    if let Ok(extra) = std::env::var("HA_TLS_NOMS") {
        for value in extra.split(',').map(str::trim).filter(|v| !v.is_empty()) {
            add(LocalName::parse(value).map_err(|error| format!("HA_TLS_NOMS : {error}"))?);
        }
    }
    Ok(names)
}

/// L'adresse de la machine sur le réseau : celle qu'elle prendrait pour
/// sortir (on « connecte » une socket UDP, rien n'est envoyé).
fn lan_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// Le nom de la machine (`maison` sur le mini-PC).
fn host_name() -> Option<String> {
    let name = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())?;
    let name = name.trim().to_ascii_lowercase();
    (!name.is_empty()).then_some(name)
}

/// Tout ce qu'il faut pour servir en HTTPS.
pub struct Setup {
    pub https: ha_api::Https,
    ca: Arc<LocalCa>,
    cert: Arc<ServerCert>,
    names: Vec<LocalName>,
}

/// Lit (ou crée) l'autorité et signe le certificat du serveur.
pub fn prepare(bind: SocketAddr) -> Result<Setup, Box<dyn Error>> {
    let dir = dir();
    let ca = LocalCa::load_or_create(&dir)?;
    let names = names(bind)?;
    let cert = ServerCert::new(ca.issue_server(&names)?);
    let config = local_ca::server_config(Arc::clone(&cert))?;
    Ok(Setup {
        https: ha_api::Https {
            config,
            ca_der: Arc::from(ca.der()),
        },
        ca: Arc::new(ca),
        cert,
        names,
    })
}

impl Setup {
    /// Le journal du démarrage : l'adresse, et le certificat à installer.
    pub fn announce(&self, bind: SocketAddr) {
        let host = self
            .names
            .iter()
            .find_map(|name| match name {
                LocalName::Ip(IpAddr::V4(ip)) if !ip.is_loopback() => Some(ip.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| "localhost".to_owned());
        let list: Vec<String> = self.names.iter().map(ToString::to_string).collect();
        info!(
            "HTTPS : https://{host}:{}/ (certificat pour {})",
            bind.port(),
            list.join(", ")
        );
        info!(
            "certificat de la maison, à installer une fois sur chaque appareil : \
             http://{host}:{}{} (empreinte SHA-256 {})",
            bind.port(),
            ha_api::CA_PATH,
            self.ca.fingerprint()
        );
    }

    /// Refait le certificat du serveur chaque semaine, avec les noms du moment.
    pub fn spawn_renewal(&self, bind: SocketAddr) {
        let ca = Arc::clone(&self.ca);
        let cert = Arc::clone(&self.cert);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(RENEW_EVERY);
            every.tick().await; // le premier vient d'être fait
            loop {
                every.tick().await;
                match names(bind).map_err(Into::into).and_then(|names| {
                    ca.issue_server(&names)
                        .map_err(|error| -> Box<dyn Error> { Box::new(error) })
                }) {
                    Ok(fresh) => {
                        cert.replace(fresh);
                        info!("HTTPS : certificat du serveur renouvelé");
                    }
                    Err(error) => warn!("HTTPS : renouvellement impossible ({error})"),
                }
            }
        });
    }
}

/// `ha-server https` : où est l'autorité, son empreinte, et comment
/// l'installer.
pub fn command() -> Result<(), Box<dyn Error>> {
    let bind = crate::parse_bind()?;
    let dir = dir();
    let ca = LocalCa::load_or_create(&dir)?;
    let names = names(bind)?;
    let list: Vec<String> = names.iter().map(ToString::to_string).collect();
    println!(
        "autorité de la maison : {}",
        dir.join(local_ca::CA_FILE).display()
    );
    println!("empreinte SHA-256     : {}", ca.fingerprint());
    println!("noms du certificat    : {}", list.join(", "));
    println!(
        "HTTPS                 : {}",
        if enabled()? {
            "activé (HA_TLS=on)"
        } else {
            "désactivé (HA_TLS=on pour l'activer)"
        }
    );
    println!(
        "à installer sur chaque appareil : http://<adresse du serveur>:{}{}",
        bind.port(),
        ha_api::CA_PATH
    );
    Ok(())
}
