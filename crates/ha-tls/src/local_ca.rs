//! Le HTTPS **sur le réseau de la maison** (étape 79), sans Internet ni nom
//! de domaine : une petite autorité de certification propre à la maison.
//!
//! - L'**autorité** (`maison-ca.crt`, valable 10 ans) est créée une fois,
//!   rangée à côté de la clé qui chiffre la base. On l'installe une fois sur
//!   chaque téléphone, tablette ou PC : ils font alors confiance au serveur.
//! - Le **certificat du serveur** est signé par elle à chaque démarrage, puis
//!   chaque semaine (valable 397 jours) : il suit l'adresse de la machine. Sa
//!   clé ne quitte jamais la mémoire.
//!
//! **Limitée à la maison** : l'autorité porte des *contraintes de nom*
//! (RFC 5280). Elle ne peut signer que des adresses privées (192.168.x.x,
//! 10.x.x.x…) et des noms en `.local`. Même volée, sa clé ne permettrait pas
//! de se faire passer pour un vrai site (une banque, Google) auprès des
//! appareils qui lui font confiance.

use std::fs;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use sha2::{Digest, Sha256};

/// Le nom du fichier de l'autorité, tel qu'on le télécharge.
pub const CA_FILE: &str = "maison-ca.crt";
const CA_KEY_FILE: &str = "maison-ca.key";

/// Durée de l'autorité : 10 ans.
const CA_DAYS: i64 = 3653;
/// Durée du certificat du serveur : 397 jours, le maximum accepté par tous
/// les navigateurs.
const SERVER_DAYS: i64 = 397;

/// Erreur de l'autorité de la maison.
#[derive(Debug, thiserror::Error)]
pub enum LocalCaError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("certificat : {0}")]
    Certificate(#[from] rcgen::Error),

    #[error("configuration TLS : {0}")]
    Tls(#[from] rustls::Error),

    #[error(
        "« {0} » : seulement des adresses privées (192.168…, 10…) et des noms en .local \
         (l'autorité de la maison ne signe rien d'autre)"
    )]
    NameNotAllowed(String),

    #[error("{0} existe mais pas {1} : rien n'est remplacé, vérifie le dossier")]
    HalfPresent(PathBuf, PathBuf),
}

/// Un nom que le certificat du serveur peut porter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LocalName {
    Ip(IpAddr),
    Dns(String),
}

impl LocalName {
    /// Lit un nom (`192.168.1.20`, `maison.local`), s'il est permis : une
    /// adresse privée, de boucle ou locale au lien, ou un nom en `.local`
    /// (ou `localhost`).
    pub fn parse(value: &str) -> Result<Self, LocalCaError> {
        let value = value.trim();
        if let Ok(ip) = value.parse::<IpAddr>() {
            return if is_private(ip) {
                Ok(Self::Ip(ip))
            } else {
                Err(LocalCaError::NameNotAllowed(value.to_owned()))
            };
        }
        let name = value.trim_end_matches('.').to_ascii_lowercase();
        let valid_label = |label: &str| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        };
        let local = name == "localhost"
            || name
                .strip_suffix(".local")
                .is_some_and(|host| host.split('.').all(valid_label));
        if local && name.len() <= 253 {
            Ok(Self::Dns(name))
        } else {
            Err(LocalCaError::NameNotAllowed(value.to_owned()))
        }
    }
}

impl std::fmt::Display for LocalName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ip(ip) => ip.fmt(f),
            Self::Dns(name) => f.write_str(name),
        }
    }
}

/// Les adresses que l'autorité peut signer (voir [`name_constraints`]).
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

/// Les contraintes de nom de l'autorité : exactement ce que [`is_private`]
/// et [`LocalName::parse`] acceptent.
fn name_constraints() -> NameConstraints {
    let v4 = |addr, prefix| GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix(addr, prefix));
    let v6 = |addr, prefix| GeneralSubtree::IpAddress(CidrSubnet::from_v6_prefix(addr, prefix));
    let unique_local = std::net::Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0).octets();
    let link_local = std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0).octets();
    NameConstraints {
        permitted_subtrees: vec![
            GeneralSubtree::DnsName("local".to_owned()),
            GeneralSubtree::DnsName("localhost".to_owned()),
            v4([10, 0, 0, 0], 8),
            v4([172, 16, 0, 0], 12),
            v4([192, 168, 0, 0], 16),
            v4([127, 0, 0, 0], 8),
            v4([169, 254, 0, 0], 16),
            v6(std::net::Ipv6Addr::LOCALHOST.octets(), 128),
            v6(unique_local, 7),
            v6(link_local, 10),
        ],
        excluded_subtrees: Vec::new(),
    }
}

/// L'autorité de certification de la maison.
pub struct LocalCa {
    cert_pem: String,
    cert_der: CertificateDer<'static>,
    issuer: Issuer<'static, KeyPair>,
}

impl std::fmt::Debug for LocalCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Jamais la clé dans un affichage.
        f.debug_struct("LocalCa")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

impl LocalCa {
    /// Lit l'autorité rangée dans `dir`, ou la crée (la clé en `0600`).
    pub fn load_or_create(dir: &Path) -> Result<Self, LocalCaError> {
        let cert_path = dir.join(CA_FILE);
        let key_path = dir.join(CA_KEY_FILE);
        match (cert_path.exists(), key_path.exists()) {
            (true, true) => {
                let cert_pem = read(&cert_path)?;
                let key = KeyPair::from_pem(&read(&key_path)?)?;
                let issuer = Issuer::from_ca_cert_pem(&cert_pem, key)?;
                let cert_der = pem_to_der(&cert_pem)?;
                Ok(Self {
                    cert_pem,
                    cert_der,
                    issuer,
                })
            }
            (false, false) => {
                let ca = Self::generate()?;
                write_private(&key_path, &ca.issuer.key().serialize_pem())?;
                write_public(&cert_path, &ca.cert_pem)?;
                Ok(ca)
            }
            (true, false) => Err(LocalCaError::HalfPresent(cert_path, key_path)),
            (false, true) => Err(LocalCaError::HalfPresent(key_path, cert_path)),
        }
    }

    /// Une nouvelle autorité, en mémoire.
    pub fn generate() -> Result<Self, LocalCaError> {
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "Maison (autorité locale)");
        name.push(DnType::OrganizationName, "home-assistant-rust");
        params.distinguished_name = name;
        // Une autorité qui signe des certificats de serveur, pas d'autres
        // autorités.
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.name_constraints = Some(name_constraints());
        set_validity(&mut params, CA_DAYS);
        let key = KeyPair::generate()?;
        let cert = params.self_signed(&key)?;
        let cert_pem = cert.pem();
        let cert_der = cert.der().clone();
        Ok(Self {
            cert_pem,
            cert_der,
            issuer: Issuer::new(params, key),
        })
    }

    /// Le certificat de l'autorité, au format DER : ce qu'on installe sur
    /// les appareils.
    #[must_use]
    pub fn der(&self) -> &[u8] {
        &self.cert_der
    }

    /// Son empreinte SHA-256 (`AB:CD:…`), pour vérifier le fichier installé.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        crate::format_fingerprint(&Sha256::digest(&self.cert_der).into())
    }

    /// Un certificat de serveur pour ces noms, signé par l'autorité, avec
    /// une clé neuve.
    pub fn issue_server(&self, names: &[LocalName]) -> Result<Arc<CertifiedKey>, LocalCaError> {
        let mut params = CertificateParams::default();
        let mut subject = DistinguishedName::new();
        subject.push(DnType::CommonName, "Maison");
        params.distinguished_name = subject;
        params.subject_alt_names = names
            .iter()
            .map(|name| match name {
                LocalName::Ip(ip) => Ok(SanType::IpAddress(*ip)),
                LocalName::Dns(name) => Ok(SanType::DnsName(name.as_str().try_into()?)),
            })
            .collect::<Result<_, rcgen::Error>>()?;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        set_validity(&mut params, SERVER_DAYS);
        let key = KeyPair::generate()?;
        let cert = params.signed_by(&key, &self.issuer)?;
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let signing = rustls::crypto::ring::sign::any_supported_type(&key_der)?;
        Ok(Arc::new(CertifiedKey::new(
            vec![cert.der().clone(), self.cert_der.clone()],
            signing,
        )))
    }
}

/// Valable de hier (une horloge un peu en retard ne gêne pas) à `days` jours.
fn set_validity(params: &mut CertificateParams, days: i64) {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let ymd = |date: jiff::civil::Date| {
        rcgen::date_time_ymd(
            i32::from(date.year()),
            date.month().unsigned_abs(),
            date.day().unsigned_abs(),
        )
    };
    let from = today.yesterday().unwrap_or(today);
    let to = today
        .checked_add(jiff::Span::new().days(days))
        .unwrap_or(today);
    params.not_before = ymd(from);
    params.not_after = ymd(to);
}

fn read(path: &Path) -> Result<String, LocalCaError> {
    fs::read_to_string(path).map_err(|source| LocalCaError::Io {
        path: path.to_owned(),
        source,
    })
}

fn pem_to_der(pem: &str) -> Result<CertificateDer<'static>, LocalCaError> {
    use rustls::pki_types::pem::PemObject as _;
    CertificateDer::from_pem_slice(pem.as_bytes())
        .map_err(|_| LocalCaError::Certificate(rcgen::Error::CouldNotParseCertificate))
}

/// Écrit la clé de l'autorité, lisible par le seul compte du serveur.
fn write_private(path: &Path, content: &str) -> Result<(), LocalCaError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut file| file.write_all(content.as_bytes()))
        .map_err(|source| LocalCaError::Io {
            path: path.to_owned(),
            source,
        })
}

fn write_public(path: &Path, content: &str) -> Result<(), LocalCaError> {
    fs::write(path, content).map_err(|source| LocalCaError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Donne à rustls le certificat du serveur du moment, qu'on peut remplacer
/// sans redémarrer (nouvelle adresse, renouvellement).
#[derive(Debug)]
pub struct ServerCert(RwLock<Arc<CertifiedKey>>);

impl ServerCert {
    #[must_use]
    pub fn new(cert: Arc<CertifiedKey>) -> Arc<Self> {
        Arc::new(Self(RwLock::new(cert)))
    }

    pub fn replace(&self, cert: Arc<CertifiedKey>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = cert;
    }
}

impl ResolvesServerCert for ServerCert {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(Arc::clone(
            &self.0.read().unwrap_or_else(PoisonError::into_inner),
        ))
    }
}

/// La configuration TLS du serveur : TLS 1.2 et 1.3, HTTP/1.1.
pub fn server_config(cert: Arc<ServerCert>) -> Result<Arc<rustls::ServerConfig>, LocalCaError> {
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_cert_resolver(cert);
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "dans un test, un échec doit faire échouer le test"
    )]

    use super::*;

    /// Un dossier vide, propre au test.
    fn empty_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ha-tls-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_home_names_are_allowed() {
        for ok in [
            "192.168.1.20",
            "10.0.0.2",
            "172.20.1.1",
            "127.0.0.1",
            "::1",
            "fd12::3",
            "maison.local",
            "Salon-TV.local.",
            "localhost",
        ] {
            assert!(LocalName::parse(ok).is_ok(), "{ok}");
        }
        for refused in [
            "8.8.8.8",
            "2a01::1",
            "google.com",
            "maison.fr",
            "local",
            "-x.local",
            "a b.local",
            "",
        ] {
            assert!(
                matches!(
                    LocalName::parse(refused),
                    Err(LocalCaError::NameNotAllowed(_))
                ),
                "{refused}"
            );
        }
        assert_eq!(
            LocalName::parse("Salon-TV.local.").unwrap(),
            LocalName::Dns("salon-tv.local".to_owned())
        );
    }

    #[test]
    fn the_authority_is_created_once_then_reused() {
        let dir = empty_dir("reuse");
        let first = LocalCa::load_or_create(&dir).unwrap();
        let again = LocalCa::load_or_create(&dir).unwrap();
        assert_eq!(first.fingerprint(), again.fingerprint());
        assert_eq!(first.fingerprint().len(), 32 * 3 - 1);
        // La clé n'est lisible que par le compte du serveur.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(dir.join(CA_KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Un certificat signé par l'autorité relue a la même racine.
        let names = [LocalName::parse("192.168.1.20").unwrap()];
        let chain = again.issue_server(&names).unwrap();
        assert_eq!(chain.cert.len(), 2);
        assert_eq!(chain.cert[1].as_ref(), first.der());
        // Jamais la clé dans un affichage de débogage.
        assert!(!format!("{first:?}").contains("PRIVATE"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn half_an_authority_is_never_replaced() {
        let dir = empty_dir("half");
        LocalCa::load_or_create(&dir).unwrap();
        fs::remove_file(dir.join(CA_KEY_FILE)).unwrap();
        assert!(matches!(
            LocalCa::load_or_create(&dir),
            Err(LocalCaError::HalfPresent(..))
        ));
        let _ = fs::remove_dir_all(dir);
    }
}
