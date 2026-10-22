//! Un petit client IMAP, juste ce qu'il faut pour lire des e-mails **sans
//! rien changer** à la boîte : `LOGIN`, `LIST`, `EXAMINE` (ouverture en
//! lecture seule), `UID SEARCH`, `UID FETCH` avec `BODY.PEEK` (le message
//! reste « non lu »), `LOGOUT`. Jamais `SELECT`, `STORE`, `COPY` ni
//! `EXPUNGE`.
//!
//! Le protocole (RFC 9051) est fait de lignes ; une donnée peut arriver
//! comme « littéral » : `{123}` en fin de ligne, puis 123 octets bruts. Les
//! tailles sont bornées ([`MAX_LINE`], [`MAX_LITERAL`], [`MAX_RESPONSE`]).

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use zeroize::Zeroizing;

use crate::ColisError;

/// Une ligne fait au plus cette taille.
const MAX_LINE: usize = 64 * 1024;
/// Un littéral (le début d'un e-mail) fait au plus cette taille.
pub const MAX_LITERAL: usize = 512 * 1024;
/// Une réponse entière fait au plus cette taille.
const MAX_RESPONSE: usize = 4 << 20;

/// Une ligne de réponse, avec ses littéraux (dans l'ordre), et le texte qui
/// précède chacun.
#[derive(Debug, Default, Clone)]
pub struct Line {
    pub text: String,
    pub literals: Vec<(String, Vec<u8>)>,
}

/// Une session IMAP, sur une connexion déjà chiffrée (ou un faux serveur
/// pour les essais).
pub struct Session<S> {
    stream: BufReader<S>,
    tag: u32,
}

/// `"texte"` en IMAP : guillemets et barres obliques échappés ; pas de fin
/// de ligne possible.
fn quoted(text: &str) -> Result<String, ColisError> {
    if text.chars().any(|c| matches!(c, '\r' | '\n' | '\0')) {
        return Err(ColisError::Refused("caractère interdit".into()));
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    /// Lit le message d'accueil du serveur.
    pub async fn start(stream: S) -> Result<Self, ColisError> {
        let mut session = Self {
            stream: BufReader::new(stream),
            tag: 0,
        };
        let mut total = 0;
        let greeting = session.read_line(&mut total).await?;
        if !greeting.text.starts_with("* OK") {
            return Err(ColisError::Protocol("accueil inattendu".into()));
        }
        Ok(session)
    }

    async fn read_line(&mut self, total: &mut usize) -> Result<Line, ColisError> {
        let mut line = Line::default();
        loop {
            let mut raw = Vec::new();
            let read = (&mut self.stream)
                .take(MAX_LINE as u64 + 2)
                .read_until(b'\n', &mut raw)
                .await
                .map_err(ColisError::Io)?;
            if read == 0 {
                return Err(ColisError::Protocol("connexion fermée".into()));
            }
            if !raw.ends_with(b"\n") {
                return Err(ColisError::Protocol("ligne trop longue".into()));
            }
            *total += raw.len();
            let piece = String::from_utf8_lossy(&raw)
                .trim_end_matches(['\r', '\n'])
                .to_owned();
            // Un littéral : `{123}` (ou `{123+}`) en fin de ligne.
            let literal = piece
                .strip_suffix('}')
                .and_then(|rest| rest.rsplit_once('{'))
                .and_then(|(before, size)| {
                    size.trim_end_matches('+')
                        .parse::<usize>()
                        .ok()
                        .map(|size| (before.to_owned(), size))
                });
            let Some((before, size)) = literal else {
                line.text.push_str(&piece);
                return Ok(line);
            };
            if size > MAX_LITERAL || *total + size > MAX_RESPONSE {
                return Err(ColisError::Protocol("réponse trop grosse".into()));
            }
            let mut data = vec![0; size];
            self.stream
                .read_exact(&mut data)
                .await
                .map_err(ColisError::Io)?;
            *total += size;
            line.text.push_str(&before);
            line.text.push_str("{…}");
            line.literals.push((before, data));
        }
    }

    /// Envoie une commande et lit la réponse jusqu'à sa ligne finale.
    /// Renvoie les lignes non étiquetées ; une réponse `NO`/`BAD` est une
    /// erreur.
    async fn command(&mut self, command: &str) -> Result<Vec<Line>, ColisError> {
        self.tag += 1;
        let tag = format!("a{}", self.tag);
        let mut sent = Zeroizing::new(format!("{tag} {command}\r\n"));
        self.stream
            .get_mut()
            .write_all(sent.as_bytes())
            .await
            .map_err(ColisError::Io)?;
        self.stream
            .get_mut()
            .flush()
            .await
            .map_err(ColisError::Io)?;
        sent.clear();
        let mut lines = Vec::new();
        let mut total = 0;
        loop {
            let line = self.read_line(&mut total).await?;
            if let Some(status) = line.text.strip_prefix(&format!("{tag} ")) {
                return if status.starts_with("OK") {
                    Ok(lines)
                } else if status.contains("AUTHENTICATIONFAILED")
                    || (command.starts_with("LOGIN") && status.starts_with("NO"))
                {
                    Err(ColisError::Unauthorized)
                } else {
                    Err(ColisError::Refused(status.chars().take(200).collect()))
                };
            }
            if !line.text.starts_with('*') && !line.text.starts_with('+') {
                return Err(ColisError::Protocol("réponse inattendue".into()));
            }
            lines.push(line);
            if lines.len() > 10_000 {
                return Err(ColisError::Protocol("réponse trop longue".into()));
            }
        }
    }

    /// Se connecte avec le compte (le mot de passe ne quitte pas la mémoire
    /// effacée après usage).
    pub async fn login(&mut self, username: &str, password: &str) -> Result<(), ColisError> {
        let command = Zeroizing::new(format!("LOGIN {} {}", quoted(username)?, quoted(password)?));
        self.command(&command).await.map(|_| ())
    }

    /// Le dossier « Tous les messages » de Gmail (marqué `\All`), où se
    /// trouvent aussi les e-mails rangés ; sinon la boîte de réception.
    pub async fn all_mail(&mut self) -> Result<String, ColisError> {
        let lines = self.command("LIST \"\" \"*\"").await?;
        for line in lines {
            let Some(rest) = line.text.strip_prefix("* LIST (") else {
                continue;
            };
            let Some((flags, after)) = rest.split_once(')') else {
                continue;
            };
            if !flags
                .split(' ')
                .any(|flag| flag.eq_ignore_ascii_case("\\All"))
            {
                continue;
            }
            if let Some((_, name)) = line.literals.first() {
                return Ok(String::from_utf8_lossy(name).into_owned());
            }
            // `"/" "[Gmail]/Tous les messages"` : le dernier mot.
            let name = after.trim();
            let name = name.rsplit_once(" \"").map_or_else(
                || name.rsplit(' ').next().unwrap_or_default().to_owned(),
                |(_, quoted)| {
                    quoted
                        .trim_end_matches('"')
                        .replace("\\\"", "\"")
                        .replace("\\\\", "\\")
                },
            );
            if !name.is_empty() {
                return Ok(name);
            }
        }
        Ok("INBOX".into())
    }

    /// Ouvre un dossier **en lecture seule**. Renvoie son `UIDVALIDITY`.
    pub async fn examine(&mut self, folder: &str) -> Result<u64, ColisError> {
        let lines = self
            .command(&format!("EXAMINE {}", quoted(folder)?))
            .await?;
        Ok(lines
            .iter()
            .find_map(|line| {
                let after = line.text.split_once("[UIDVALIDITY ")?.1;
                after.split(']').next()?.trim().parse().ok()
            })
            .unwrap_or(0))
    }

    /// Les messages reçus depuis `since` (« 8-Oct-2026 ») dont l'expéditeur
    /// contient `from`.
    pub async fn search(&mut self, since: &str, from: &str) -> Result<Vec<u32>, ColisError> {
        if !since.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(ColisError::Refused("date invalide".into()));
        }
        let lines = self
            .command(&format!("UID SEARCH SINCE {since} FROM {}", quoted(from)?))
            .await?;
        let mut uids: Vec<u32> = lines
            .iter()
            .filter_map(|line| line.text.strip_prefix("* SEARCH"))
            .flat_map(|rest| rest.split_whitespace().filter_map(|n| n.parse().ok()))
            .collect();
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
    }

    /// Les en-têtes utiles et le début du texte d'un message, sans le
    /// marquer comme lu (`BODY.PEEK`).
    pub async fn fetch(&mut self, uid: u32) -> Result<Option<(Vec<u8>, Vec<u8>)>, ColisError> {
        let lines = self
            .command(&format!(
                "UID FETCH {uid} (BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE AUTHENTICATION-RESULTS)] \
                 BODY.PEEK[TEXT]<0.{}>)",
                MAX_LITERAL - 1024
            ))
            .await?;
        for line in lines {
            if !line.text.contains("FETCH") {
                continue;
            }
            let mut headers = None;
            let mut text = Vec::new();
            for (before, data) in line.literals {
                if before.contains("HEADER") {
                    headers = Some(data);
                } else if before.contains("TEXT") {
                    text = data;
                }
            }
            if let Some(headers) = headers {
                return Ok(Some((headers, text)));
            }
        }
        Ok(None)
    }

    /// Se déconnecte proprement.
    pub async fn logout(&mut self) {
        let _ = self.command("LOGOUT").await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn strings_are_quoted_safely() {
        assert_eq!(quoted("a\"b\\c").unwrap(), r#""a\"b\\c""#);
        assert!(quoted("a\r\nA2 DELETE INBOX").is_err());
    }

    /// Une conversation complète avec un faux serveur, dans un tube.
    #[tokio::test]
    async fn a_read_only_session() {
        let (client, server) = tokio::io::duplex(1 << 20);
        let fake = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut read = BufReader::new(read);
            write.write_all(b"* OK Gimap ready\r\n").await.unwrap();
            let mut seen = Vec::new();
            loop {
                let mut line = String::new();
                if read.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                seen.push(command.to_owned());
                let reply: Vec<u8> = if command.starts_with("LOGIN") {
                    format!("{tag} OK connecté\r\n").into()
                } else if command.starts_with("LIST") {
                    format!(
                        "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n\
                         * LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/Tous les messages\"\r\n{tag} OK\r\n"
                    )
                    .into()
                } else if command.starts_with("EXAMINE") {
                    format!("* OK [UIDVALIDITY 42] ok\r\n{tag} OK [READ-ONLY]\r\n").into()
                } else if command.starts_with("UID SEARCH") {
                    format!("* SEARCH 7 3\r\n{tag} OK\r\n").into()
                } else if command.starts_with("UID FETCH") {
                    let headers = "Subject: Livré : « Plaid »\r\n\r\n";
                    let text = "Bonjour";
                    format!(
                        "* 1 FETCH (UID 7 BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{headers} BODY[TEXT]<0> {{{}}}\r\n{text})\r\n{tag} OK\r\n",
                        headers.len(),
                        text.len()
                    )
                    .into()
                } else if command.starts_with("LOGOUT") {
                    format!("* BYE\r\n{tag} OK\r\n").into()
                } else {
                    format!("{tag} BAD inconnu\r\n").into()
                };
                write.write_all(&reply).await.unwrap();
                if command.starts_with("LOGOUT") {
                    break;
                }
            }
            seen
        });
        let mut session = Session::start(client).await.unwrap();
        session
            .login("maison@gmail.com", "mot \"passe\"")
            .await
            .unwrap();
        let folder = session.all_mail().await.unwrap();
        assert_eq!(folder, "[Gmail]/Tous les messages");
        assert_eq!(session.examine(&folder).await.unwrap(), 42);
        assert_eq!(
            session.search("8-Oct-2026", "amazon").await.unwrap(),
            [3, 7]
        );
        let (headers, text) = session.fetch(7).await.unwrap().unwrap();
        assert!(String::from_utf8(headers).unwrap().contains("Plaid"));
        assert_eq!(text, b"Bonjour");
        session.logout().await;
        let seen = fake.await.unwrap();
        assert_eq!(seen[0], r#"LOGIN "maison@gmail.com" "mot \"passe\"""#);
        // Jamais rien qui modifie la boîte.
        for command in &seen {
            for forbidden in ["SELECT", "STORE", "COPY", "EXPUNGE", "DELETE", "BODY["] {
                assert!(
                    !command.starts_with(forbidden) && !command.contains(&format!("({forbidden}")),
                    "{command}"
                );
            }
        }
        assert!(seen.iter().any(|c| c.contains("BODY.PEEK[TEXT]")));
    }
}
