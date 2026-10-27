//! Lire le corps d'une réponse HTTP **sans dépasser une taille**.
//!
//! `response.bytes()` de reqwest lit tout, quelle que soit la taille : un
//! serveur (ou un appareil) défaillant pourrait nous faire remplir la
//! mémoire. Ici, on lit morceau par morceau et on s'arrête dès que la
//! limite est dépassée. Toutes les intégrations passent par là.

/// Pourquoi une réponse n'a pas pu être lue en entier.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// La connexion a échoué pendant la lecture.
    ///
    /// Concept Rust : `#[error(transparent)]` reprend tel quel le message de
    /// l'erreur d'origine, et `#[from]` permet `?` sur une `reqwest::Error`.
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    /// Plus de `max` octets.
    #[error("réponse trop grosse")]
    TooLarge,
}

/// Lit tout le corps de `response`, `max` octets au plus.
///
/// Une réponse qui annonce d'avance une taille trop grande
/// (`Content-Length`) est refusée sans rien lire.
pub async fn read_limited(
    mut response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, ReadError> {
    let max_u64 = u64::try_from(max).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > max_u64)
    {
        return Err(ReadError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > max {
            return Err(ReadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    /// Une réponse dont la taille est connue d'avance.
    fn whole(body: &[u8]) -> reqwest::Response {
        http::Response::new(body.to_vec()).into()
    }

    /// Une réponse en plusieurs morceaux, sans taille annoncée.
    fn chunked(chunks: &[&[u8]]) -> reqwest::Response {
        let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
            chunks.iter().map(|chunk| Ok(chunk.to_vec())).collect();
        let body = reqwest::Body::wrap_stream(futures_util::stream::iter(chunks));
        let response = reqwest::Response::from(http::Response::new(body));
        assert_eq!(response.content_length(), None);
        response
    }

    #[tokio::test]
    async fn reads_up_to_the_limit() {
        assert_eq!(
            read_limited(whole(b"bonjour"), 7).await.unwrap(),
            b"bonjour"
        );
        assert_eq!(read_limited(whole(b""), 0).await.unwrap(), b"");
        let body = read_limited(chunked(&[b"bon", b"jour"]), 7).await.unwrap();
        assert_eq!(body, b"bonjour");
    }

    #[tokio::test]
    async fn refuses_a_bigger_body() {
        let error = read_limited(whole(b"bonjour"), 6).await.unwrap_err();
        assert!(matches!(error, ReadError::TooLarge));
        let error = read_limited(chunked(&[b"bon", b"jour"]), 6)
            .await
            .unwrap_err();
        assert!(matches!(error, ReadError::TooLarge));
        assert_eq!(error.to_string(), "réponse trop grosse");
    }
}
