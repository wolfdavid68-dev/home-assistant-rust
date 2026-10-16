//! La cryptographie du Web Push, avec `ring` :
//!
//! - **le chiffrement du contenu** (RFC 8291, `aes128gcm`) : seul le
//!   navigateur abonné peut lire la notification. Le service qui la
//!   transporte (Google, Mozilla, Apple…) ne voit qu'un bloc chiffré ;
//! - **la signature VAPID** (RFC 8292) : un jeton JWT signé par la clé du
//!   serveur. Le navigateur s'est abonné avec notre clé publique : le service
//!   refuse tout envoi qui n'est pas signé par la clé privée correspondante.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{ECDH_P256, EphemeralPrivateKey, UnparsedPublicKey, agree_ephemeral};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use zeroize::Zeroizing;

/// Une clé publique P-256 « non compressée » : 0x04, puis x et y.
pub const PUBLIC_KEY_LEN: usize = 65;
/// La même longueur, telle qu'écrite dans l'en-tête (un octet).
const PUBLIC_KEY_LEN_BYTE: u8 = 65;
/// Le secret « auth » du navigateur.
pub const AUTH_LEN: usize = 16;
/// Taille d'un enregistrement chiffré (un seul pour nous).
const RECORD_SIZE: u32 = 4096;
const SALT_LEN: usize = 16;
const TAG_LEN: usize = 16;
/// En-tête : sel, taille d'enregistrement, longueur et valeur de la clé.
const HEADER_LEN: usize = SALT_LEN + 4 + 1 + PUBLIC_KEY_LEN;
/// Le plus long contenu qui tient dans un enregistrement (moins le
/// délimiteur de fin).
pub const MAX_PLAINTEXT: usize = RECORD_SIZE as usize - HEADER_LEN - TAG_LEN - 1;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("clé publique du navigateur invalide")]
    PublicKey,
    #[error("secret « auth » du navigateur invalide")]
    Auth,
    #[error("notification trop longue")]
    TooLong,
    #[error("clé VAPID invalide")]
    Vapid,
    #[error("erreur de chiffrement")]
    Crypto,
}

impl From<ring::error::Unspecified> for CryptoError {
    fn from(_: ring::error::Unspecified) -> Self {
        Self::Crypto
    }
}

/// base64url sans « = », comme le veulent le Web Push et les JWT.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Lit du base64url, avec ou sans « = » final (les navigateurs varient).
#[must_use]
pub fn decode(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// Une longueur de sortie HKDF quelconque (`ring` demande un type).
struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn hkdf<const N: usize>(salt: &[u8], ikm: &[u8], info: &[&[u8]]) -> Result<[u8; N], CryptoError> {
    let prk = Salt::new(HKDF_SHA256, salt).extract(ikm);
    let mut out = [0u8; N];
    prk.expand(info, Len(N))?.fill(&mut out)?;
    Ok(out)
}

/// La clé et le nonce du contenu (RFC 8291, section 3.4), à partir du
/// secret partagé ECDH.
fn content_keys(
    ecdh_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    auth: &[u8],
    salt: &[u8],
) -> Result<(Zeroizing<[u8; 16]>, [u8; 12]), CryptoError> {
    let ikm = Zeroizing::new(hkdf::<32>(
        auth,
        ecdh_secret,
        &[b"WebPush: info\0", ua_public, as_public],
    )?);
    let cek = Zeroizing::new(hkdf::<16>(
        salt,
        ikm.as_slice(),
        &[b"Content-Encoding: aes128gcm\0"],
    )?);
    let nonce = hkdf::<12>(salt, ikm.as_slice(), &[b"Content-Encoding: nonce\0"])?;
    Ok((cek, nonce))
}

/// Assemble le message chiffré : en-tête, puis contenu chiffré.
fn seal(
    ecdh_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    auth: &[u8],
    salt: &[u8; SALT_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let (cek, nonce) = content_keys(ecdh_secret, ua_public, as_public, auth, salt)?;
    let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, cek.as_slice())?);
    let mut record = Vec::with_capacity(plaintext.len() + 1 + TAG_LEN);
    record.extend_from_slice(plaintext);
    // Délimiteur : c'est le dernier (et le seul) enregistrement.
    record.push(2);
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::empty(),
        &mut record,
    )?;
    let mut body = Vec::with_capacity(HEADER_LEN + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(PUBLIC_KEY_LEN_BYTE);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&record);
    Ok(body)
}

/// Chiffre `plaintext` pour le navigateur qui a donné `ua_public` (sa clé
/// « p256dh ») et `auth`. Une nouvelle clé éphémère et un nouveau sel à
/// chaque message.
pub fn encrypt(ua_public: &[u8], auth: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if ua_public.len() != PUBLIC_KEY_LEN || ua_public.first() != Some(&4) {
        return Err(CryptoError::PublicKey);
    }
    if auth.len() != AUTH_LEN {
        return Err(CryptoError::Auth);
    }
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(CryptoError::TooLong);
    }
    let rng = SystemRandom::new();
    let private = EphemeralPrivateKey::generate(&ECDH_P256, &rng)?;
    let as_public = private.compute_public_key()?;
    let mut salt = [0u8; SALT_LEN];
    rng.fill(&mut salt)?;
    let peer = UnparsedPublicKey::new(&ECDH_P256, ua_public);
    let ecdh_secret = agree_ephemeral(private, &peer, |secret| Zeroizing::new(secret.to_vec()))
        .map_err(|_| CryptoError::PublicKey)?;
    seal(
        &ecdh_secret,
        ua_public,
        as_public.as_ref(),
        auth,
        &salt,
        plaintext,
    )
}

/// La paire de clés VAPID du serveur.
pub struct Vapid {
    key: EcdsaKeyPair,
    public: String,
    rng: SystemRandom,
}

impl std::fmt::Debug for Vapid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vapid")
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}

impl Vapid {
    /// Une nouvelle paire : la clé privée (PKCS#8) à ranger, et la paire.
    pub fn generate() -> Result<(Zeroizing<Vec<u8>>, Self), CryptoError> {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)?;
        let pkcs8 = Zeroizing::new(pkcs8.as_ref().to_vec());
        let vapid = Self::from_pkcs8(&pkcs8)?;
        Ok((pkcs8, vapid))
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, CryptoError> {
        let rng = SystemRandom::new();
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8, &rng)
            .map_err(|_| CryptoError::Vapid)?;
        let public = encode(key.public_key().as_ref());
        Ok(Self { key, public, rng })
    }

    /// La clé publique, en base64url (pour `applicationServerKey`).
    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public
    }

    /// L'en-tête `Authorization` d'un envoi vers `audience` (l'origine du
    /// service : `https://fcm.googleapis.com`), valable jusqu'à `expires`
    /// (secondes depuis 1970, 24 h au plus).
    pub fn authorization(
        &self,
        audience: &str,
        expires: u64,
        contact: &str,
    ) -> Result<String, CryptoError> {
        let header = encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({ "aud": audience, "exp": expires, "sub": contact });
        let claims = encode(claims.to_string().as_bytes());
        let signing_input = format!("{header}.{claims}");
        let signature = self.key.sign(&self.rng, signing_input.as_bytes())?;
        Ok(format!(
            "vapid t={signing_input}.{}, k={}",
            encode(signature.as_ref()),
            self.public
        ))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey as VerifyKey};

    fn b(text: &str) -> Vec<u8> {
        decode(text).unwrap()
    }

    /// L'exemple de la RFC 8291 (annexe A), étape par étape.
    #[test]
    fn the_rfc_8291_example() {
        let ua_public = b(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let as_public = b(
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        let auth = b("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = b("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let ecdh_secret = b("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");

        let (cek, nonce) =
            content_keys(&ecdh_secret, &ua_public, &as_public, &auth, &salt).unwrap();
        assert_eq!(encode(cek.as_slice()), "oIhVW04MRdy2XN9CiKLxTg");
        assert_eq!(encode(&nonce), "4h_95klXJ5E_qnoN");

        let body = seal(
            &ecdh_secret,
            &ua_public,
            &as_public,
            &auth,
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(
            encode(&body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    /// Ce que fait le navigateur : déchiffrer avec sa clé privée.
    pub(crate) fn decrypt(
        ua_private: EphemeralPrivateKey,
        ua_public: &[u8],
        auth: &[u8],
        body: &[u8],
    ) -> Vec<u8> {
        let salt = &body[..16];
        assert_eq!(&body[16..20], &4096u32.to_be_bytes());
        assert_eq!(body[20], 65);
        let as_public = &body[21..86];
        let ecdh_secret = agree_ephemeral(
            ua_private,
            &UnparsedPublicKey::new(&ECDH_P256, as_public),
            <[u8]>::to_vec,
        )
        .unwrap();
        let (cek, nonce) = content_keys(&ecdh_secret, ua_public, as_public, auth, salt).unwrap();
        let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, cek.as_slice()).unwrap());
        let mut record = body[86..].to_vec();
        let plain = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut record,
            )
            .unwrap();
        assert_eq!(plain.last(), Some(&2), "délimiteur de fin");
        plain[..plain.len() - 1].to_vec()
    }

    pub(crate) fn browser() -> (EphemeralPrivateKey, Vec<u8>, Vec<u8>) {
        let rng = SystemRandom::new();
        let private = EphemeralPrivateKey::generate(&ECDH_P256, &rng).unwrap();
        let public = private.compute_public_key().unwrap().as_ref().to_vec();
        let mut auth = vec![0u8; 16];
        rng.fill(&mut auth).unwrap();
        (private, public, auth)
    }

    #[test]
    fn a_browser_reads_what_we_encrypt() {
        let (private, public, auth) = browser();
        let message = "Ordre à valider : fermer le volet de la chambre".as_bytes();
        let body = encrypt(&public, &auth, message).unwrap();
        assert_eq!(decrypt(private, &public, &auth, &body), message);
        // Deux envois du même texte ne se ressemblent pas.
        assert_ne!(
            encrypt(&public, &auth, message).unwrap(),
            encrypt(&public, &auth, message).unwrap()
        );
    }

    #[test]
    fn bad_keys_and_long_messages_are_refused() {
        let (_, public, auth) = browser();
        assert_eq!(
            encrypt(&public[1..], &auth, b"x").unwrap_err(),
            CryptoError::PublicKey
        );
        let mut not_on_curve = public.clone();
        not_on_curve[10] ^= 0xff;
        assert_eq!(
            encrypt(&not_on_curve, &auth, b"x").unwrap_err(),
            CryptoError::PublicKey
        );
        assert_eq!(
            encrypt(&public, &auth[..8], b"x").unwrap_err(),
            CryptoError::Auth
        );
        assert!(encrypt(&public, &auth, &vec![b'a'; MAX_PLAINTEXT]).is_ok());
        assert_eq!(
            encrypt(&public, &auth, &vec![b'a'; MAX_PLAINTEXT + 1]).unwrap_err(),
            CryptoError::TooLong
        );
    }

    #[test]
    fn the_vapid_token_is_signed_by_our_key() {
        let (pkcs8, vapid) = Vapid::generate().unwrap();
        let again = Vapid::from_pkcs8(&pkcs8).unwrap();
        assert_eq!(vapid.public_key(), again.public_key());
        assert_eq!(decode(vapid.public_key()).unwrap().len(), 65);
        assert!(Vapid::from_pkcs8(b"pas une cle").is_err());

        let header = vapid
            .authorization("https://fcm.googleapis.com", 1_700_000_000, "mailto:a@b.fr")
            .unwrap();
        let rest = header.strip_prefix("vapid t=").unwrap();
        let (token, key) = rest.split_once(", k=").unwrap();
        assert_eq!(key, vapid.public_key());
        let (signed, signature) = token.rsplit_once('.').unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&decode(signed.split_once('.').unwrap().1).unwrap()).unwrap();
        assert_eq!(
            claims,
            serde_json::json!({"aud": "https://fcm.googleapis.com", "exp": 1_700_000_000, "sub": "mailto:a@b.fr"})
        );
        let verify = VerifyKey::new(&ECDSA_P256_SHA256_FIXED, decode(key).unwrap());
        assert!(
            verify
                .verify(signed.as_bytes(), &decode(signature).unwrap())
                .is_ok()
        );
        assert!(
            verify
                .verify(b"autre chose", &decode(signature).unwrap())
                .is_err()
        );
    }
}
