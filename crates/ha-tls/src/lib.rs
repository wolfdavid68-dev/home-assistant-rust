//! Connexions HTTPS, partagées par les intégrations (Hue, Somfy…).
//!
//! - [`client`] : vers un **appareil du réseau local** (pont Hue, box
//!   TaHoma), avec **épinglage du certificat** ;
//! - [`public_client`] : vers un **service sur Internet**, avec la
//!   vérification habituelle par les autorités de confiance du système.
//!
//! Pourquoi l'épinglage ? Les appareils locaux utilisent des certificats qui
//! ne sont pas signés par une autorité publique (comme ceux des sites web) :
//! un navigateur afficherait un avertissement. Beaucoup de programmes
//! désactivent alors simplement la vérification… ce qui permettrait à
//! n'importe quel appareil du réseau de se faire passer pour le pont et de
//! récupérer la clé d'accès.
//!
//! On fait mieux : lors de l'association (`ha-server hue pair`…), on
//! mémorise l'empreinte SHA-256 du certificat de l'appareil. Ensuite, **seul
//! ce certificat précis est accepté**. C'est le principe « confiance au
//! premier usage » (TOFU), le même que SSH quand il demande « The
//! authenticity of host … ».

mod body;
pub mod local_ca;

pub use body::{ReadError, read_limited};

use std::sync::{Arc, Mutex, PoisonError};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

use rustls_platform_verifier::BuilderVerifierExt;

/// Erreur de préparation ou d'utilisation d'une connexion HTTPS.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("configuration TLS impossible : {0}")]
    Tls(#[from] rustls::Error),

    #[error("client HTTPS impossible : {0}")]
    Http(#[from] reqwest::Error),
}

/// Empreinte SHA-256 d'un certificat.
pub type Fingerprint = [u8; 32];

/// Vérificateur de certificat par empreinte.
///
/// Concept Rust : on implémente le trait `ServerCertVerifier` de rustls.
/// rustls appelle nos méthodes pendant la connexion, et refuse la connexion
/// si l'une d'elles renvoie une erreur.
#[derive(Debug)]
pub struct PinnedVerifier {
    /// `Some` : seul ce certificat est accepté. `None` : mode association,
    /// on accepte et on note l'empreinte vue.
    expected: Option<Fingerprint>,
    seen: Mutex<Option<Fingerprint>>,
    provider: Arc<CryptoProvider>,
}

impl PinnedVerifier {
    /// Le certificat vu lors de la dernière connexion (mode association).
    pub fn seen(&self) -> Option<Fingerprint> {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fingerprint: Fingerprint = Sha256::digest(end_entity.as_ref()).into();
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(fingerprint);
        match self.expected {
            Some(expected) if expected != fingerprint => Err(rustls::Error::General(
                "certificat inattendu : ce n'est peut-être pas ton appareil (pont, box…)".into(),
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    // Les deux fonctions suivantes vérifient que le pont possède bien la clé
    // privée de son certificat : sans elles, épingler le certificat (public)
    // ne servirait à rien, n'importe qui pourrait le présenter.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Crée un client HTTPS qui n'accepte que le certificat `expected`
/// (ou n'importe lequel, en le notant, si `expected` vaut `None`).
pub fn client(
    expected: Option<Fingerprint>,
) -> Result<(reqwest::Client, Arc<PinnedVerifier>), TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(PinnedVerifier {
        expected,
        seen: Mutex::new(None),
        provider: Arc::clone(&provider),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        // `dangerous()` : rustls nous rappelle qu'on remplace sa vérification
        // standard. C'est voulu, et notre vérification est plus stricte.
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&verifier) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(config)
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()?;
    Ok((client, verifier))
}

/// Crée un client HTTPS pour un service sur Internet (serveur Somfy…).
///
/// Le certificat est vérifié comme le ferait un navigateur, avec les
/// autorités de confiance **du système** (magasin de certificats de
/// Windows, de macOS ou de Linux).
pub fn public_client() -> Result<reqwest::Client, TlsError> {
    web_client(true)
}

/// Comme [`public_client`], mais en acceptant aussi le HTTP non chiffré si
/// `https_only` est faux. À réserver aux adresses du réseau local (un
/// serveur de notifications chez toi, par exemple) : c'est à l'appelant de
/// le vérifier.
pub fn web_client(https_only: bool) -> Result<reqwest::Client, TlsError> {
    let config = public_config()?;
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(config)
        .https_only(https_only)
        // Pas de redirection suivie : une réponse ne peut pas nous envoyer
        // (avec nos identifiants) vers une autre adresse.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// La configuration TLS vers un service sur Internet : certificat vérifié
/// par les autorités de confiance du système. Sert aussi hors HTTP (la
/// boîte aux lettres des colis, en IMAP sur TLS, étape 87).
pub fn public_config() -> Result<rustls::ClientConfig, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_platform_verifier()?
        .with_no_client_auth())
}

/// Vérifie une adresse d'appareil : une adresse IPv4 ou un nom d'hôte
/// simple, éventuellement suivi de `:port`.
///
/// Sécurité : elle finit dans une URL (`https://<hôte>/…`) ; on refuse donc
/// tout caractère qui pourrait en changer le sens (`/`, `@`, `?`…).
#[must_use]
pub fn is_valid_host(host: &str) -> bool {
    let (name, port) = match host.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port.is_none_or(|port| port.parse::<u16>().is_ok_and(|p| p > 0))
}

/// Empreinte au format lisible : `AB:CD:…`.
#[must_use]
pub fn format_fingerprint(fingerprint: &Fingerprint) -> String {
    fingerprint
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
