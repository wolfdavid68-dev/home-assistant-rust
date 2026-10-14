//! Connexion HTTPS au pont Hue, avec **épinglage du certificat**.
//!
//! Le pont Hue utilise un certificat qui n'est pas signé par une autorité
//! publique (comme ceux des sites web) : un navigateur afficherait un
//! avertissement. Beaucoup de programmes désactivent alors simplement la
//! vérification… ce qui permettrait à n'importe quel appareil du réseau de se
//! faire passer pour le pont et de récupérer la clé d'accès.
//!
//! On fait mieux : lors de l'association (`ha-server hue pair`), on mémorise
//! l'empreinte SHA-256 du certificat du pont. Ensuite, **seul ce certificat
//! précis est accepté**. C'est le principe « confiance au premier usage »
//! (TOFU), le même que SSH quand il demande « The authenticity of host … ».

use std::sync::{Arc, Mutex, PoisonError};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::error::HueError;

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
                "certificat du pont Hue inattendu : ce n'est peut-être pas ton pont".into(),
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
) -> Result<(reqwest::Client, Arc<PinnedVerifier>), HueError> {
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

/// Empreinte au format lisible : `AB:CD:…`.
pub fn format_fingerprint(fingerprint: &Fingerprint) -> String {
    fingerprint
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
