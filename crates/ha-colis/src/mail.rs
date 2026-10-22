//! Lire un e-mail : ses en-têtes (avec les mots encodés de la RFC 2047),
//! son texte (« quoted-printable » ou base64), et vérifier qu'il vient
//! **vraiment** d'Amazon (signature DKIM vérifiée par la boîte aux lettres).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Les domaines d'Amazon dont on accepte les e-mails.
const AMAZON_DOMAINS: &[&str] = &[
    "amazon.fr",
    "amazon.com",
    "amazon.de",
    "amazon.co.uk",
    "amazon.it",
    "amazon.es",
    "amazon.nl",
    "amazon.com.be",
    "amazon.ca",
    "amazon.ch",
    "amazon.lu",
    "amazon.pl",
    "amazon.se",
    "amazon.ie",
];

/// Les serveurs d'images d'Amazon : la seule origine d'où l'on télécharge
/// la photo d'un article.
pub const IMAGE_HOSTS: &[&str] = &[
    "m.media-amazon.com",
    "images-eu.ssl-images-amazon.com",
    "images-na.ssl-images-amazon.com",
];

/// Les en-têtes utiles d'un e-mail.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Headers {
    pub from: String,
    pub subject: String,
    pub date: String,
    /// Le **premier** `Authentication-Results` : celui que la boîte aux
    /// lettres a ajouté en recevant le message (ceux d'en dessous peuvent
    /// venir de l'expéditeur).
    pub authentication: Option<String>,
}

impl Headers {
    /// Lit les en-têtes (lignes repliées comprises).
    #[must_use]
    pub fn parse(raw: &[u8]) -> Self {
        let text = String::from_utf8_lossy(raw);
        let mut fields: Vec<(String, String)> = Vec::new();
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                break;
            }
            if line.starts_with([' ', '\t']) {
                if let Some((_, value)) = fields.last_mut() {
                    value.push(' ');
                    value.push_str(line.trim());
                }
            } else if let Some((name, value)) = line.split_once(':') {
                fields.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
            }
        }
        let first = |name: &str| {
            fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, value)| value.clone())
        };
        Self {
            from: decode_words(&first("from").unwrap_or_default()),
            subject: decode_words(&first("subject").unwrap_or_default()),
            date: first("date").unwrap_or_default(),
            authentication: first("authentication-results"),
        }
    }

    /// L'adresse de l'expéditeur : `Amazon <livraison@amazon.fr>` →
    /// `livraison@amazon.fr`.
    #[must_use]
    pub fn sender(&self) -> String {
        let from = self.from.trim();
        let address = from
            .rsplit_once('<')
            .and_then(|(_, rest)| rest.split_once('>'))
            .map_or(from, |(address, _)| address);
        address.trim().to_ascii_lowercase()
    }

    /// Vient-il d'Amazon ? L'expéditeur est sur un domaine d'Amazon, **et**
    /// la boîte aux lettres a vérifié la signature DKIM d'Amazon (ou DMARC).
    /// Sans ce contrôle, n'importe qui pourrait écrire « Livré : … » en se
    /// faisant passer pour Amazon.
    #[must_use]
    pub fn is_from_amazon(&self) -> bool {
        let sender = self.sender();
        let Some((_, domain)) = sender.rsplit_once('@') else {
            return false;
        };
        if !is_amazon_domain(domain) {
            return false;
        }
        let Some(results) = &self.authentication else {
            return false;
        };
        let results = results.to_ascii_lowercase();
        // `dkim=pass header.i=@amazon.fr header.s=…` ; `dmarc=pass (…) header.from=amazon.fr`.
        results.split(';').any(|clause| {
            let clause = clause.trim();
            let passed = clause.starts_with("dkim=pass") || clause.starts_with("dmarc=pass");
            passed
                && clause.split_whitespace().any(|word| {
                    ["header.i=", "header.d=", "header.from="]
                        .iter()
                        .find_map(|key| word.strip_prefix(key))
                        .is_some_and(|value| {
                            let value = value.trim_start_matches('@').trim_end_matches(')');
                            let domain = value.rsplit_once('@').map_or(value, |(_, d)| d);
                            is_amazon_domain(domain)
                        })
                })
        })
    }
}

/// `amazon.fr`, ou un sous-domaine (`marketplace.amazon.fr`).
#[must_use]
pub fn is_amazon_domain(domain: &str) -> bool {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    AMAZON_DOMAINS.iter().any(|amazon| {
        domain == *amazon
            || domain
                .strip_suffix(amazon)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// Les mots encodés d'un en-tête : `=?UTF-8?Q?Livr=C3=A9?=` → « Livré ».
#[must_use]
pub fn decode_words(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    let mut after_word = false;
    while let Some(start) = rest.find("=?") {
        let (before, candidate) = rest.split_at(start);
        if let Some((text, used)) = parse_word(candidate) {
            // Les blancs entre deux mots encodés ne comptent pas.
            if !(after_word && before.trim().is_empty()) {
                out.push_str(before);
            }
            out.push_str(&text);
            rest = &candidate[used..];
            after_word = true;
        } else {
            out.push_str(before);
            out.push_str("=?");
            rest = &candidate[2..];
            after_word = false;
        }
    }
    out.push_str(rest);
    out
}

/// Un mot encodé au début de `text` : son texte, et sa longueur.
fn parse_word(text: &str) -> Option<(String, usize)> {
    let inner = text.strip_prefix("=?")?;
    let (charset, inner) = inner.split_once('?')?;
    let (encoding, inner) = inner.split_once('?')?;
    let end = inner.find("?=")?;
    let payload = &inner[..end];
    if payload.contains(' ') {
        return None;
    }
    let bytes = match encoding {
        "Q" | "q" => quoted_printable(payload.replace('_', " ").as_bytes()),
        "B" | "b" => STANDARD.decode(payload).ok()?,
        _ => return None,
    };
    let used = 2 + charset.len() + 1 + encoding.len() + 1 + end + 2;
    // La langue éventuelle : `UTF-8*fr`.
    let charset = charset.split('*').next().unwrap_or_default();
    Some((decode_charset(charset, &bytes), used))
}

fn decode_charset(charset: &str, bytes: &[u8]) -> String {
    let charset = charset.to_ascii_lowercase();
    if charset.contains("8859") || charset.contains("1252") || charset == "latin1" {
        bytes.iter().map(|&b| char::from(b)).collect()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Décode le « quoted-printable » : `=C3=A9` → é, `=` en fin de ligne →
/// ligne continuée.
#[must_use]
pub fn quoted_printable(input: &[u8]) -> Vec<u8> {
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'=' {
            match (input.get(i + 1), input.get(i + 2)) {
                (Some(b'\r'), Some(b'\n')) => i += 3,
                (Some(b'\n'), _) => i += 2,
                (Some(&a), Some(&b)) => {
                    if let (Some(a), Some(b)) = (hex(a), hex(b)) {
                        out.push(u8::try_from(a * 16 + b).unwrap_or(b'?'));
                        i += 3;
                    } else {
                        out.push(b'=');
                        i += 1;
                    }
                }
                _ => {
                    out.push(b'=');
                    i += 1;
                }
            }
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    out
}

/// Le texte d'un e-mail, lisible : les parties en base64 décodées, le
/// reste décodé du « quoted-printable ».
#[must_use]
pub fn readable_text(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::from_utf8_lossy(&quoted_printable(raw)).into_owned();
    // Les parties en base64 : leur en-tête, une ligne vide, puis les lignes
    // de base64.
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("content-transfer-encoding: base64") {
        let start = from + found;
        from = start + 1;
        let Some(blank) = text[start..]
            .find("\n\r\n")
            .or_else(|| text[start..].find("\n\n"))
        else {
            break;
        };
        let body = &text[start + blank..];
        let encoded: String = body
            .lines()
            .map(str::trim)
            .skip_while(|line| line.is_empty())
            .take_while(|line| {
                !line.is_empty()
                    && line
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
            })
            .collect();
        if let Ok(bytes) = STANDARD.decode(encoded) {
            out.push('\n');
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    out
}

/// La photo du premier article : `https://m.media-amazon.com/images/I/….jpg`
/// (les logos sont sous `/images/G/`, on les laisse).
#[must_use]
pub fn product_image(text: &str) -> Option<String> {
    let mut from = 0;
    while let Some(found) = text[from..].find("https://") {
        let start = from + found;
        from = start + 8;
        let url: String = text[start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | ')' | '('))
            .collect();
        let Some(rest) = url.strip_prefix("https://") else {
            continue;
        };
        let Some((host, path)) = rest.split_once('/') else {
            continue;
        };
        let image = std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("png"));
        if IMAGE_HOSTS.contains(&host)
            && path.starts_with("images/I/")
            && image
            && url.len() < 300
            && !path.contains("..")
            && path.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+' | b',')
            })
        {
            return Some(url);
        }
    }
    None
}

/// Le numéro de commande Amazon : `405-1234567-1234567`.
#[must_use]
pub fn order_number(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let shape = [3, 7, 7];
    (0..bytes.len()).find_map(|start| {
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            return None;
        }
        let mut at = start;
        for (index, len) in shape.iter().enumerate() {
            if !bytes.get(at..at + len)?.iter().all(u8::is_ascii_digit) {
                return None;
            }
            at += len;
            if index < 2 {
                if bytes.get(at) != Some(&b'-') {
                    return None;
                }
                at += 1;
            }
        }
        if bytes.get(at).is_some_and(u8::is_ascii_digit) {
            return None;
        }
        Some(text[start..at].to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_words_are_decoded() {
        assert_eq!(
            decode_words("=?UTF-8?Q?Livr=C3=A9_:_=C2=AB_Plaid_=C2=BB?="),
            "Livré : « Plaid »"
        );
        assert_eq!(
            decode_words("=?utf-8?B?RXhww6lkacOpIDo=?= =?utf-8?Q?_Plaid?="),
            "Expédié : Plaid"
        );
        assert_eq!(decode_words("=?ISO-8859-1?Q?exp=E9di=E9?="), "expédié");
        assert_eq!(decode_words("a =?x"), "a =?x");
    }

    #[test]
    fn headers_are_unfolded() {
        let raw = b"From: \"Amazon.fr\" <livraison@amazon.fr>\r\nSubject: =?UTF-8?Q?Livr=C3=A9?=\r\n =?UTF-8?Q?_:_Plaid?=\r\nAuthentication-Results: mx.google.com;\r\n       dkim=pass header.i=@amazon.fr header.s=abc;\r\n       spf=pass\r\nAuthentication-Results: faux; dkim=pass header.i=@amazon.fr\r\n\r\n";
        let headers = Headers::parse(raw);
        assert_eq!(headers.subject, "Livré : Plaid");
        assert_eq!(headers.sender(), "livraison@amazon.fr");
        assert!(
            headers
                .authentication
                .as_deref()
                .unwrap()
                .starts_with("mx.google.com")
        );
        assert!(headers.is_from_amazon());
    }

    #[test]
    fn only_signed_amazon_mail_is_trusted() {
        let mail = |from: &str, auth: Option<&str>| Headers {
            from: from.into(),
            authentication: auth.map(Into::into),
            ..Headers::default()
        };
        let good = "mx.google.com; dkim=pass header.i=@amazon.fr header.s=x; spf=pass";
        assert!(mail("Amazon <a@amazon.fr>", Some(good)).is_from_amazon());
        assert!(
            mail(
                "<a@marketplace.amazon.fr>",
                Some("x; dmarc=pass (p=QUARANTINE) header.from=marketplace.amazon.fr")
            )
            .is_from_amazon()
        );
        // Pas de signature, ou une signature ratée, ou d'un autre domaine.
        assert!(!mail("a@amazon.fr", None).is_from_amazon());
        assert!(!mail("a@amazon.fr", Some("x; dkim=fail header.i=@amazon.fr")).is_from_amazon());
        assert!(
            !mail("a@amazon.fr", Some("x; dkim=pass header.i=@pirate.example")).is_from_amazon()
        );
        // Des domaines qui ressemblent.
        for from in [
            "a@notamazon.fr",
            "a@amazon.fr.pirate.example",
            "a@amazon-fr.com",
            "amazon.fr",
        ] {
            assert!(!mail(from, Some(good)).is_from_amazon(), "{from}");
        }
    }

    #[test]
    fn quoted_printable_and_base64_bodies() {
        assert_eq!(
            quoted_printable(b"caf=C3=A9 =\r\nau lait"),
            "café au lait".as_bytes()
        );
        let raw = b"--b\r\nContent-Type: text/html\r\nContent-Transfer-Encoding: base64\r\n\r\nPGltZyBzcmM9Imh0dHBzOi8vbS5tZWRpYS1hbWF6b24uY29tL2ltYWdlcy9JLzcxYWJjLl9TUzEzNV8uanBnIj4=\r\n--b--\r\n";
        let text = readable_text(raw);
        assert_eq!(
            product_image(&text).as_deref(),
            Some("https://m.media-amazon.com/images/I/71abc._SS135_.jpg")
        );
    }

    #[test]
    fn product_images_come_from_amazon_only() {
        let html = r#"<img src="https://m.media-amazon.com/images/G/08/logo.png"><img src="https://pirate.example/images/I/x.jpg"><img src="https://m.media-amazon.com/images/I/81Xy-z._AC_SX300_.jpg">"#;
        assert_eq!(
            product_image(html).as_deref(),
            Some("https://m.media-amazon.com/images/I/81Xy-z._AC_SX300_.jpg")
        );
        assert_eq!(
            product_image("https://m.media-amazon.com/images/I/a.gif"),
            None
        );
        assert_eq!(
            product_image("https://m.media-amazon.com/images/I/a%2F..jpg"),
            None
        );
    }

    #[test]
    fn order_numbers() {
        assert_eq!(
            order_number("Commande n° 405-1234567-7654321 du 20 octobre").as_deref(),
            Some("405-1234567-7654321")
        );
        assert_eq!(order_number("1405-1234567-7654321"), None);
        assert_eq!(order_number("rien"), None);
    }
}
