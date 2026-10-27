//! Chiffrement des secrets rangés dans la base : clé du pont Hue, jeton de
//! la box Somfy…
//!
//! Sans chiffrement, une copie de `ha.db` (une sauvegarde oubliée sur une
//! clé USB, par exemple) suffirait pour piloter tes appareils. Les secrets
//! sont donc chiffrés avec une clé rangée dans **un autre fichier**
//! (`ha.key`). La base seule ne permet plus rien.
//!
//! Algorithme : XChaCha20-Poly1305 (chiffrement « authentifié » : toute
//! modification du texte chiffré est détectée). Chaque secret a son propre
//! nonce aléatoire de 24 octets, et il est lié à son usage (« contexte ») :
//! on ne peut pas recopier le jeton Somfy à la place de la clé Hue.
//!
//! Format en base : `enc:v1:` suivi de nonce + texte chiffré, en hexadécimal.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::StorageError;

const PREFIX: &str = "enc:v1:";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
/// Taille maximale d'un secret déchiffré : bien plus que nécessaire.
const MAX_SEALED_LEN: usize = 16 * 1024;

/// La clé de chiffrement des secrets.
///
/// Concept Rust : `Arc` permet de partager la clé entre toutes les copies
/// du `Store` sans la dupliquer en mémoire. Le chiffreur efface sa clé de la
/// mémoire quand il est libéré (option `zeroize` de la crate).
#[derive(Clone)]
pub struct SecretKey(Arc<Keys>);

struct Keys {
    cipher: XChaCha20Poly1305,
    /// Clé de signature du journal d'audit, dérivée de la clé principale.
    audit: Zeroizing<[u8; 32]>,
}

/// On n'affiche jamais la clé, même dans un message de débogage.
impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(<masquée>)")
    }
}

impl SecretKey {
    /// Clé aléatoire, jamais enregistrée (base en mémoire, tests).
    pub fn random() -> Result<Self, StorageError> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|_| StorageError::Secret("aléatoire"))?;
        Self::from_bytes(bytes.as_slice())
    }

    /// Lit la clé dans le fichier `path`, ou la crée s'il n'existe pas.
    ///
    /// Le fichier est créé lisible par toi seul (droits 600 sous Linux).
    pub fn load_or_create(path: &Path) -> Result<Self, StorageError> {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let content = Zeroizing::new(content);
                let bytes = Zeroizing::new(
                    decode_hex(content.trim())
                        .filter(|bytes| bytes.len() == KEY_LEN)
                        .ok_or(StorageError::Secret(
                            "fichier de clé invalide (64 caractères hexadécimaux attendus)",
                        ))?,
                );
                Self::from_bytes(&bytes)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::create(path),
            Err(error) => Err(error.into()),
        }
    }

    fn create(path: &Path) -> Result<Self, StorageError> {
        use std::io::Write;

        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|_| StorageError::Secret("aléatoire"))?;
        let mut options = std::fs::OpenOptions::new();
        // `create_new` : on n'écrase jamais une clé existante.
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        let mut text = Zeroizing::new(encode_hex(bytes.as_slice()));
        text.push('\n');
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        tracing::info!("nouvelle clé de chiffrement créée : {}", path.display());
        Self::from_bytes(bytes.as_slice())
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, StorageError> {
        let cipher = XChaCha20Poly1305::new_from_slice(bytes)
            .map_err(|_| StorageError::Secret("taille de clé"))?;
        // Une clé par usage : la clé du journal est dérivée de la clé
        // principale (HMAC avec une étiquette), au lieu de la réutiliser.
        let mut derive = <Hmac<Sha256> as Mac>::new_from_slice(bytes)
            .map_err(|_| StorageError::Secret("taille de clé"))?;
        derive.update(b"ha-rust audit log v1");
        let audit = Zeroizing::new(derive.finalize().into_bytes().into());
        Ok(Self(Arc::new(Keys { cipher, audit })))
    }

    /// Signature (HMAC-SHA256) d'une ligne du journal d'audit. Chaque champ
    /// est précédé de sa longueur : « ab » + « c » et « a » + « bc » ne
    /// donnent pas la même signature.
    pub(crate) fn audit_mac(&self, fields: &[&[u8]]) -> [u8; 32] {
        let Ok(mut mac) = <Hmac<Sha256> as Mac>::new_from_slice(self.0.audit.as_slice()) else {
            // HMAC accepte des clés de toute taille : ce cas n'arrive pas.
            return [0; 32];
        };
        for field in fields {
            mac.update(&(field.len() as u64).to_be_bytes());
            mac.update(field);
        }
        mac.finalize().into_bytes().into()
    }

    /// Chiffre `plaintext`, pour l'usage `context` (par exemple
    /// `"hue_bridge.app_key"`).
    pub fn seal(&self, context: &str, plaintext: &str) -> Result<String, StorageError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| StorageError::Secret("aléatoire"))?;
        let ciphertext = self
            .0
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| StorageError::Secret("chiffrement impossible"))?;
        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(format!("{PREFIX}{}", encode_hex(&sealed)))
    }

    /// Déchiffre un secret produit par [`SecretKey::seal`] avec le même
    /// `context`. Échoue si la clé n'est pas la bonne ou si le secret a été
    /// modifié.
    pub fn unseal(&self, context: &str, sealed: &str) -> Result<Zeroizing<String>, StorageError> {
        let hex = sealed
            .strip_prefix(PREFIX)
            .filter(|hex| hex.len() <= 2 * MAX_SEALED_LEN)
            .ok_or(StorageError::Secret("secret non chiffré ou trop long"))?;
        let bytes = decode_hex(hex).ok_or(StorageError::Secret("secret illisible"))?;
        let (nonce, ciphertext) = bytes
            .split_first_chunk::<NONCE_LEN>()
            .ok_or(StorageError::Secret("secret trop court"))?;
        let plaintext = Zeroizing::new(
            self.0
                .cipher
                .decrypt(
                    &XNonce::from(*nonce),
                    Payload {
                        msg: ciphertext,
                        aad: context.as_bytes(),
                    },
                )
                .map_err(|_| {
                    StorageError::Secret(
                        "secret indéchiffrable : clé de chiffrement différente, ou base modifiée",
                    )
                })?,
        );
        let text = std::str::from_utf8(&plaintext)
            .map_err(|_| StorageError::Secret("secret illisible"))?;
        Ok(Zeroizing::new(text.to_owned()))
    }
}

/// Le secret est-il chiffré ? (Les associations faites avant le chiffrement
/// sont encore en clair : elles sont chiffrées à la première lecture.)
#[must_use]
pub fn is_sealed(value: &str) -> bool {
    value.starts_with(PREFIX)
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    // Concept Rust : `as_chunks::<2>()` découpe en paires d'octets de taille
    // connue à la compilation, plus un reste (vide si la longueur est paire).
    let (pairs, rest) = text.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|&[high, low]| {
            let high = char::from(high).to_digit(16)?;
            let low = char::from(low).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = SecretKey::random().unwrap();
        let sealed = key.seal("somfy_box.token", "jeton-secret").unwrap();
        assert!(is_sealed(&sealed));
        assert!(!sealed.contains("jeton-secret"));
        assert_eq!(
            key.unseal("somfy_box.token", &sealed).unwrap().as_str(),
            "jeton-secret"
        );
    }

    #[test]
    fn same_secret_twice_gives_different_ciphertexts() {
        let key = SecretKey::random().unwrap();
        let a = key.seal("ctx", "même texte").unwrap();
        let b = key.seal("ctx", "même texte").unwrap();
        assert_ne!(a, b, "chaque chiffrement a son propre nonce");
    }

    #[test]
    fn wrong_key_or_context_is_refused() {
        let key = SecretKey::random().unwrap();
        let other = SecretKey::random().unwrap();
        let sealed = key.seal("hue_bridge.app_key", "clé").unwrap();
        assert!(other.unseal("hue_bridge.app_key", &sealed).is_err());
        assert!(key.unseal("somfy_box.token", &sealed).is_err());
    }

    #[test]
    fn tampering_is_detected() {
        let key = SecretKey::random().unwrap();
        let sealed = key.seal("ctx", "clé").unwrap();
        // On modifie le dernier caractère (une partie du code d'authentification).
        let mut tampered = sealed.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        assert!(key.unseal("ctx", &tampered).is_err());
        assert!(key.unseal("ctx", "clé en clair").is_err());
        assert!(key.unseal("ctx", "enc:v1:zz").is_err());
        assert!(key.unseal("ctx", "enc:v1:00").is_err());
    }

    #[test]
    fn key_file_is_created_then_reused() {
        let dir = std::env::temp_dir().join(format!("ha-key-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ha.key");
        let _ = std::fs::remove_file(&path);

        let first = SecretKey::load_or_create(&path).unwrap();
        let sealed = first.seal("ctx", "secret").unwrap();
        let again = SecretKey::load_or_create(&path).unwrap();
        assert_eq!(again.unseal("ctx", &sealed).unwrap().as_str(), "secret");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        std::fs::write(&path, "pas une clé").unwrap();
        assert!(SecretKey::load_or_create(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn debug_never_shows_the_key() {
        let key = SecretKey::random().unwrap();
        assert_eq!(format!("{key:?}"), "SecretKey(<masquée>)");
    }

    #[test]
    fn hex_round_trip() {
        let bytes = [0u8, 1, 0x7f, 0x80, 0xff];
        assert_eq!(decode_hex(&encode_hex(&bytes)).unwrap(), bytes);
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("gg"), None);
    }
}
