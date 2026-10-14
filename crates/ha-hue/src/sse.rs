//! Lecture d'un flux « Server-Sent Events » : du texte où chaque événement
//! est un bloc de lignes `data: …`, séparé du suivant par une ligne vide.

/// Taille maximale d'un événement en attente : au-delà, le flux est jugé
/// défaillant et on se reconnecte.
const MAX_BUFFER: usize = 1024 * 1024;

/// Découpe les morceaux reçus du réseau en événements complets.
///
/// Concept : le réseau livre les données par morceaux de taille quelconque :
/// un événement peut arriver coupé en deux, ou plusieurs dans un seul morceau.
/// On accumule donc dans un tampon et on ne rend que les événements complets.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("événement trop gros dans le flux")]
pub struct TooLarge;

impl SseParser {
    /// Ajoute un morceau et renvoie le contenu `data` des événements complets.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, TooLarge> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((end, separator_len)) = find_separator(&self.buffer) {
            let raw: Vec<u8> = self.buffer.drain(..end + separator_len).collect();
            let text = String::from_utf8_lossy(&raw[..end]);
            let data: Vec<&str> = text
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect();
            // Les lignes commençant par `:` sont des commentaires (`: hi`).
            if !data.is_empty() {
                events.push(data.join("\n"));
            }
        }
        if self.buffer.len() > MAX_BUFFER {
            self.buffer.clear();
            return Err(TooLarge);
        }
        Ok(events)
    }
}

/// Position de la première ligne vide (`\n\n` ou `\r\n\r\n`).
fn find_separator(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_split_across_chunks() {
        let mut parser = SseParser::default();
        assert_eq!(
            parser.push(b": hi\n\nid: 1\nda").unwrap_or_default(),
            Vec::<String>::new()
        );
        assert_eq!(
            parser
                .push(b"ta: [1]\n\nid: 2\ndata: [2]\n\n")
                .unwrap_or_default(),
            ["[1]", "[2]"]
        );
    }

    #[test]
    fn crlf_and_multiline_data() {
        let mut parser = SseParser::default();
        assert_eq!(
            parser
                .push(b"data: a\r\ndata: b\r\n\r\n")
                .unwrap_or_default(),
            ["a\nb"]
        );
    }

    #[test]
    fn oversized_event_is_rejected() {
        let mut parser = SseParser::default();
        let big = vec![b'x'; MAX_BUFFER + 1];
        assert_eq!(parser.push(&big), Err(TooLarge));
    }
}
