//! Les colis de bout en bout, contre un faux Gmail (IMAP) et un faux
//! serveur d'images d'Amazon, sur ce PC.

#![allow(clippy::unwrap_used, clippy::panic, reason = "tests")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ha_colis::{Colis, Mailbox};
use ha_core::{Core, Event};
use ha_storage::{MailAccount, Store};
use jiff::{ToSpan, Zoned};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

const PASSWORD: &str = "abcdefghijklmnop";
const SIGNED: &str =
    "mx.google.com;\r\n       dkim=pass header.i=@amazon.fr header.s=x;\r\n       spf=pass";

#[derive(Default)]
struct Fake {
    /// (uid, en-têtes, texte).
    mails: Vec<(u32, String, String)>,
    commands: Vec<String>,
}

fn mail(subject: &str, auth: Option<&str>, ago: jiff::Span, text: &str) -> (String, String) {
    let date = Zoned::now().checked_sub(ago).unwrap();
    let mut headers = format!(
        "From: \"Amazon.fr\" <livraison@amazon.fr>\r\nSubject: {subject}\r\nDate: {}\r\n",
        jiff::fmt::rfc2822::to_string(&date).unwrap()
    );
    if let Some(auth) = auth {
        headers.push_str("Authentication-Results: ");
        headers.push_str(auth);
        headers.push_str("\r\n");
    }
    headers.push_str("\r\n");
    (headers, text.to_owned())
}

/// Un faux Gmail : il ne connaît que ce que la maison doit envoyer.
async fn fake_gmail(fake: Arc<Mutex<Fake>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let fake = Arc::clone(&fake);
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut read = BufReader::new(read);
                write.write_all(b"* OK Gimap ready\r\n").await.unwrap();
                loop {
                    let mut line = String::new();
                    if read.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    fake.lock().unwrap().commands.push(command.to_owned());
                    let reply = if let Some(rest) = command.strip_prefix("LOGIN ") {
                        if rest == format!("\"maison@gmail.com\" \"{PASSWORD}\"") {
                            format!("{tag} OK maison@gmail.com authenticated\r\n")
                        } else {
                            format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")
                        }
                    } else if command.starts_with("LIST") {
                        format!(
                            "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n\
                             * LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/Tous les messages\"\r\n{tag} OK\r\n"
                        )
                    } else if command == "EXAMINE \"[Gmail]/Tous les messages\"" {
                        format!(
                            "* OK [UIDVALIDITY 7] UIDs valid\r\n{tag} OK [READ-ONLY] EXAMINE completed\r\n"
                        )
                    } else if command.starts_with("UID SEARCH SINCE ")
                        && command.ends_with("FROM \"amazon\"")
                    {
                        let uids: Vec<String> = fake
                            .lock()
                            .unwrap()
                            .mails
                            .iter()
                            .map(|m| m.0.to_string())
                            .collect();
                        format!("* SEARCH {}\r\n{tag} OK\r\n", uids.join(" "))
                    } else if let Some(rest) = command.strip_prefix("UID FETCH ") {
                        let uid: u32 = rest.split(' ').next().unwrap().parse().unwrap();
                        let found = fake
                            .lock()
                            .unwrap()
                            .mails
                            .iter()
                            .find(|m| m.0 == uid)
                            .cloned();
                        match found {
                            Some((uid, headers, text)) => format!(
                                "* {uid} FETCH (UID {uid} BODY[HEADER.FIELDS (FROM SUBJECT DATE AUTHENTICATION-RESULTS)] {{{}}}\r\n{headers} BODY[TEXT]<0> {{{}}}\r\n{text})\r\n{tag} OK\r\n",
                                headers.len(),
                                text.len()
                            ),
                            None => format!("{tag} OK\r\n"),
                        }
                    } else if command == "LOGOUT" {
                        write
                            .write_all(format!("* BYE\r\n{tag} OK\r\n").as_bytes())
                            .await
                            .unwrap();
                        return;
                    } else {
                        format!("{tag} BAD inconnu\r\n")
                    };
                    write.write_all(reply.as_bytes()).await.unwrap();
                }
            });
        }
    });
    port
}

/// Un faux serveur d'images : un petit JPEG pour toute demande.
async fn fake_images(asked: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let asked = Arc::clone(&asked);
            tokio::spawn(async move {
                let mut buffer = vec![0; 4096];
                let read = stream.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                asked
                    .lock()
                    .unwrap()
                    .push(request.split(' ').nth(1).unwrap_or_default().to_owned());
                let body = [
                    0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 0xFF, 0xD9,
                ];
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            });
        }
    });
    port
}

async fn wait_for(
    core: &Core,
    entity: &str,
    check: impl Fn(&ha_core::State) -> bool,
) -> ha_core::State {
    for _ in 0..200 {
        if let Some(state) = core.states.get(&entity.parse().unwrap())
            && check(&state)
        {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "{entity} n'a jamais eu l'état attendu : {:?}",
        core.states.get(&entity.parse().unwrap())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_parcel_on_its_way_then_delivered() {
    let fake = Arc::new(Mutex::new(Fake::default()));
    let shipped_text = "Bonjour,\r\nCommande n=C2=B0 405-1234567-7654321\r\n<img src=3D\"https://m.media-amazon.com/images/G/08/logo.png\">\r\n<img src=3D\"https://m.media-amazon.com/images/I/71plaid._SS135_.jpg\">\r\n";
    {
        let mut fake = fake.lock().unwrap();
        let (h, t) = mail(
            "=?UTF-8?Q?Exp=C3=A9di=C3=A9_:_=C2=AB_Plaid_Polaire_Doux_150x200_cm,_Couverture..._=C2=BB?=",
            Some(SIGNED),
            2.days(),
            shipped_text,
        );
        fake.mails.push((11, h, t));
        // Un faux « Amazon » sans signature : ignoré.
        let (h, t) = mail(
            "Livré : « Carte cadeau, appelez le 0800 »",
            None,
            1.hour(),
            "",
        );
        fake.mails.push((12, h, t));
        let (h, t) = mail(
            "Votre colis Amazon arrive aujourd'hui",
            Some(SIGNED),
            10.minutes(),
            "",
        );
        fake.mails.push((13, h, t));
    }
    let imap = fake_gmail(Arc::clone(&fake)).await;
    let asked = Arc::new(Mutex::new(Vec::new()));
    let images = fake_images(Arc::clone(&asked)).await;
    let mailbox = Mailbox::local(
        &format!("127.0.0.1:{imap}"),
        Some(&format!("http://127.0.0.1:{images}")),
    )
    .unwrap();

    let store = Store::open_in_memory().await.unwrap();
    let account = MailAccount {
        host: "imap.gmail.com".into(),
        port: 993,
        username: "maison@gmail.com".into(),
        password: Zeroizing::new("mauvais".into()),
    };
    // Mauvais mot de passe : refusé clairement.
    assert!(matches!(
        ha_colis::check(&mailbox, &account).await,
        Err(ha_colis::ColisError::Unauthorized)
    ));
    let account = MailAccount {
        password: Zeroizing::new(PASSWORD.into()),
        ..account
    };
    let report = ha_colis::check(&mailbox, &account).await.unwrap();
    assert_eq!(report.parcels.len(), 1, "{report:?}");
    assert_eq!((report.mails, report.unsigned), (3, 1));
    store.save_mail_account(&account).await.unwrap();

    let core = Arc::new(Core::new());
    let mut events = core.bus.subscribe();
    let colis = Colis::start(store.clone(), mailbox, Arc::clone(&core));
    let sensor = wait_for(&core, "sensor.colis_amazon", |s| s.state() == "1").await;
    let attributes = sensor.attributes();
    assert_eq!(attributes["aujourdhui"], 1);
    let last = &attributes["dernier"];
    assert_eq!(last["etat"], "en_livraison");
    assert_eq!(last["article"], "Plaid Polaire Doux");
    assert_eq!(last["phrase"], "Plaid Polaire Doux va arriver aujourd'hui.");
    assert_eq!(last["id"], "405-1234567-7654321");
    assert_eq!(last["image"], true);
    let camera = wait_for(&core, "camera.colis_amazon", |_| true).await;
    assert_eq!(camera.attributes()["colis"], "405-1234567-7654321");
    assert!(
        core.images
            .get(&"camera.colis_amazon".parse().unwrap())
            .is_some()
    );
    assert_eq!(
        asked.lock().unwrap().as_slice(),
        ["/images/I/71plaid._SS135_.jpg"]
    );

    // Livré : un évènement, le compteur retombe.
    {
        let mut fake = fake.lock().unwrap();
        let (h, t) = mail(
            "Livré : « Plaid Polaire Doux 150x200 cm, Couverture... »",
            Some(SIGNED),
            1.minute(),
            "Commande n° 405-1234567-7654321",
        );
        fake.mails.push((14, h, t));
    }
    let sensor = wait_for(&core, "sensor.colis_amazon", |s| s.state() == "0").await;
    assert_eq!(sensor.attributes()["livres_aujourdhui"], 1);
    assert_eq!(
        sensor.attributes()["dernier"]["phrase"],
        "Plaid Polaire Doux a été livré."
    );
    let mut fired = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Event::Fired { event_type, data } = event.as_ref() {
            fired.push((event_type.clone(), data["article"].clone()));
        }
    }
    // Rien au premier relevé (l'état de départ) ; « livré » ensuite.
    assert_eq!(
        fired,
        [(
            "colis_livre".to_owned(),
            serde_json::json!("Plaid Polaire Doux")
        )]
    );
    colis.stop().await;

    let commands = fake.lock().unwrap().commands.clone();
    // Lecture seule : jamais SELECT, STORE, COPY, EXPUNGE ; toujours PEEK.
    for command in &commands {
        let verb = command.split(' ').next().unwrap();
        assert!(
            ["LOGIN", "LIST", "EXAMINE", "UID", "LOGOUT"].contains(&verb),
            "{command}"
        );
        if command.starts_with("UID FETCH") {
            assert!(
                command.contains("BODY.PEEK[HEADER.FIELDS") && !command.contains(" BODY["),
                "{command}"
            );
        }
    }
    // Chaque e-mail n'est téléchargé qu'une fois par le serveur.
    let fetched = |uid: u32| {
        commands
            .iter()
            .filter(|c| c.starts_with(&format!("UID FETCH {uid} ")))
            .count()
    };
    // (1 pour `check` avec le bon mot de passe, 1 pour la boucle.)
    assert_eq!(fetched(11), 2);
    assert_eq!(fetched(14), 1);
}
