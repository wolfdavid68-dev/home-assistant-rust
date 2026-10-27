//! `ha-server smartthings | colis | timetree | iglou …` : les comptes en
//! ligne de la maison (SmartThings, boîte des colis, TimeTree, Iglou),
//! gardés chiffrés dans la base.

use std::error::Error;

use ha_storage::Store;
use zeroize::Zeroizing;

use super::USAGE;
use super::audit::audit_cli;
use super::terminal::{ask_line, ask_secret, local_time};

/// `smartthings pair|status|devices|forget` : la maison SmartThings de
/// Samsung (étape 86).
pub(crate) async fn smartthings_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = smartthings_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "smartthings", args).await;
    }
    store.close().await;
    result
}

async fn smartthings_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    use ha_smartthings::api::{self, Api, App};

    let endpoint = crate::smartthings_endpoint();
    let http = endpoint.http()?;
    match args {
        ["pair"] => {
            println!("Il faut l'application SmartThings créée pour la maison (une seule fois,");
            println!(
                "avec « smartthings apps:create », voir docs/smartthings.md) : son Client ID,"
            );
            println!("son Client Secret et l'adresse de retour (redirect URI) qu'elle connaît.");
            println!();
            let client_id = ask_line("Client ID : ")?;
            if client_id.is_empty()
                || client_id.len() > 128
                || !client_id.chars().all(|c| c.is_ascii_graphic())
            {
                return Err("Client ID invalide".into());
            }
            let client_secret = ask_secret("Client Secret : ")?;
            if client_secret.is_empty() || client_secret.len() > 256 {
                return Err("Client Secret invalide".into());
            }
            let redirect_uri = ask_line("Adresse de retour (redirect URI) : ")?;
            if !redirect_uri.starts_with("https://")
                || redirect_uri.len() > 512
                || !redirect_uri.chars().all(|c| c.is_ascii_graphic())
            {
                return Err("l'adresse de retour doit commencer par https://".into());
            }
            let mut random = [0u8; 16];
            getrandom::fill(&mut random)
                .map_err(|error| format!("aléatoire indisponible : {error}"))?;
            let state: String = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .concat();
            println!();
            println!("1. Ouvre cette adresse, connecte-toi à ton compte Samsung, choisis le lieu");
            println!("   et autorise « la maison » :");
            println!();
            println!(
                "{}",
                api::authorize_url(&endpoint, &client_id, &redirect_uri, &state)
            );
            println!();
            println!("2. Tu arrives sur l'adresse de retour (la page peut afficher une erreur :");
            println!("   ce n'est pas grave). Copie l'adresse complète de cette page.");
            let pasted = ask_line("Colle-la ici : ")?;
            let code = api::read_callback(&pasted, &state)?;
            let app = App {
                client_id,
                client_secret,
                redirect_uri,
            };
            let tokens = app.exchange(&http, &endpoint, &code).await?;
            let devices = Api::new(http.clone(), endpoint.clone(), tokens.access_token.clone())
                .devices()
                .await?;
            store
                .save_smartthings_account(&ha_storage::SmartThingsAccount {
                    client_id: app.client_id,
                    client_secret: app.client_secret,
                    redirect_uri: app.redirect_uri,
                    tokens,
                })
                .await?;
            println!("associé !");
            print_smartthings_devices(devices);
            println!("Relance le serveur (sudo systemctl restart ha-maison) pour les voir.");
            Ok(())
        }
        ["status" | "devices"] => {
            let Some(account) = store.smartthings_account().await? else {
                println!("aucune maison SmartThings associée (ha-server smartthings pair)");
                return Ok(());
            };
            println!("application : {}", account.client_id);
            println!("adresse de retour : {}", account.redirect_uri);
            if let Some((refreshed, paired)) = store.smartthings_dates().await? {
                println!("associée le {}", local_time(paired));
                println!("jetons renouvelés le {}", local_time(refreshed));
            }
            let left = account
                .tokens
                .expires_at
                .duration_since(std::time::SystemTime::now())
                .unwrap_or_default();
            // On ne renouvelle pas les jetons ici : c'est le serveur qui
            // s'en charge (un jeton de renouvellement ne sert qu'une fois).
            if left.is_zero() {
                println!(
                    "jeton d'accès expiré : le serveur le renouvellera à son prochain passage"
                );
                return Ok(());
            }
            println!("jeton d'accès valable encore {} h", left.as_secs() / 3600);
            if args == ["devices"] {
                let devices = Api::new(http, endpoint, account.tokens.access_token)
                    .devices()
                    .await?;
                print_smartthings_devices(devices);
            }
            Ok(())
        }
        ["forget"] => {
            if store.forget_smartthings_account().await? {
                println!(
                    "association SmartThings oubliée. Pour couper aussi l'accès côté Samsung :"
                );
                println!("« smartthings apps:delete » sur l'application de la maison.");
            } else {
                println!("aucune maison SmartThings associée");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Les appareils et les entités qu'ils donneront.
fn print_smartthings_devices(devices: Vec<ha_smartthings::api::Device>) {
    let core = ha_core::Core::new();
    let mut mapped = ha_smartthings::devices::Devices::default();
    let total = devices.len();
    mapped.sync(devices, &std::collections::HashMap::new(), &core.states);
    println!("{total} appareils, dont {} utilisables :", mapped.len());
    for (label, entities) in mapped.summary() {
        println!("  {label} : {}", entities.join(", "));
    }
}

/// `colis pair|status|forget` : la boîte aux lettres où arrivent les
/// e-mails d'Amazon (étape 87).
pub(crate) async fn colis_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = colis_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "colis", args).await;
    }
    store.close().await;
    result
}

async fn colis_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    let mailbox = crate::colis_mailbox();
    match args {
        ["pair"] => {
            println!("La maison lit les e-mails d'Amazon (lecture seule) pour suivre les colis.");
            println!("Il faut un « mot de passe d'application » Google, pas ton mot de passe");
            println!("habituel : voir docs/colis.md (myaccount.google.com/apppasswords).");
            println!();
            let host = ask_line("Serveur IMAP [imap.gmail.com] : ")?;
            let host = if host.is_empty() {
                "imap.gmail.com".to_owned()
            } else {
                host
            };
            if !ha_tls::is_valid_host(&host) || host.contains(':') {
                return Err("serveur invalide".into());
            }
            let username = ask_line("Adresse e-mail : ")?;
            if !username.contains('@')
                || username.len() > 254
                || !username.chars().all(|c| c.is_ascii_graphic())
            {
                return Err("adresse e-mail invalide".into());
            }
            if is_microsoft_mailbox(&host, &username) {
                return Err(MICROSOFT_MAILBOX.into());
            }
            let pasted = ask_secret("Mot de passe d'application (16 lettres) : ")?;
            // Google l'affiche en quatre groupes : les espaces ne comptent pas.
            let password = Zeroizing::new(pasted.split_whitespace().collect::<String>());
            if password.is_empty() || password.len() > 256 {
                return Err("mot de passe invalide".into());
            }
            let account = ha_storage::MailAccount {
                host,
                port: 993,
                username,
                password,
            };
            println!("connexion…");
            let report =
                ha_colis::check(&mailbox, &account)
                    .await
                    .map_err(|error| match error {
                        ha_colis::ColisError::Unauthorized => {
                            "compte refusé : vérifie l'adresse et le mot de passe d'application"
                                .to_owned()
                        }
                        other => other.to_string(),
                    })?;
            store.save_mail_account(&account).await?;
            println!("associée !");
            print_report(&report);
            println!();
            println!("Redémarre le serveur : sudo systemctl restart ha-maison");
            Ok(())
        }
        ["status"] => {
            let Some(account) = store.mail_account().await? else {
                println!("aucune boîte associée (ha-server colis pair)");
                return Ok(());
            };
            println!("boîte : {} sur {}", account.username, account.host);
            let report = ha_colis::check(&mailbox, &account).await?;
            print_report(&report);
            Ok(())
        }
        ["forget"] => {
            if store.forget_mail_account().await? {
                println!("boîte oubliée. Pour couper aussi l'accès côté Google : supprime le mot");
                println!("de passe d'application sur myaccount.google.com/apppasswords.");
            } else {
                println!("aucune boîte associée");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Hotmail, Outlook.com, Live, MSN : Microsoft refuse les mots de passe
/// aux programmes (IMAP), il faut passer par une redirection vers Gmail.
const MICROSOFT_MAILBOX: &str = "\
Hotmail / Outlook.com n'accepte plus de mot de passe pour lire la boîte depuis un
programme (Microsoft exige une application enregistrée chez Azure).
Le plus simple : une règle Hotmail qui redirige les e-mails d'Amazon vers une
boîte Gmail, et associer cette boîte Gmail ici. Voir docs/colis.md, « Avec Hotmail ».";

fn is_microsoft_mailbox(host: &str, username: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let domain = username
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_ascii_lowercase())
        .unwrap_or_default();
    host.contains("outlook")
        || host.contains("office365")
        || host.contains("hotmail")
        || ["hotmail.", "outlook.", "live.", "msn."]
            .iter()
            .any(|prefix| domain.starts_with(prefix))
}

/// Ce que la boîte montre : les colis, et les e-mails ignorés.
fn print_report(report: &ha_colis::Report) {
    print_parcels(&report.parcels);
    if report.unsigned > 0 {
        println!();
        println!(
            "attention : {} e-mail(s) « d'Amazon » sur {} ignoré(s) : la signature d'Amazon",
            report.unsigned, report.mails
        );
        println!(
            "n'a pas été vérifiée (un faux, ou un transfert qui l'a cassée : voir docs/colis.md)."
        );
    }
}

/// Les colis des deux dernières semaines.
fn print_parcels(parcels: &[ha_colis::amazon::Parcel]) {
    if parcels.is_empty() {
        println!("aucun colis Amazon ces deux dernières semaines");
        return;
    }
    println!("colis Amazon des deux dernières semaines :");
    for parcel in parcels.iter().rev() {
        let name = ha_colis::amazon::short_name(&parcel.article);
        let when = parcel
            .at
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%d/%m %H:%M");
        println!("  {when}  {}", parcel.step.sentence(&name));
    }
}

/// `timetree pair|status|forget` : le compte TimeTree de la maison
/// (étape 74).
pub(crate) async fn timetree_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = timetree_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "timetree", args).await;
    }
    store.close().await;
    result
}

fn timetree_client(
    account: ha_storage::TimeTreeAccount,
) -> Result<ha_ics::timetree::Client, Box<dyn Error>> {
    let api = crate::timetree_api();
    // Le faux TimeTree des essais est en HTTP, sur ce PC.
    let http = if api.starts_with("http://127.0.0.1:") {
        ha_tls::web_client(false)?
    } else {
        ha_tls::public_client()?
    };
    Ok(ha_ics::timetree::Client::new(http, &api, account))
}

fn print_timetree_calendars(calendars: &[ha_ics::timetree::CalendarInfo]) {
    println!("agendas visibles par ce compte :");
    for calendar in calendars {
        println!("  - {}", calendar.name);
    }
    println!();
    println!("Pour en lire un, ajoute dans calendriers.yaml :");
    println!("  - nom: Famille");
    println!(
        "    timetree: {}",
        calendars.first().map_or("Famille", |c| c.name.as_str())
    );
}

async fn timetree_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    match args {
        ["pair"] => {
            println!(
                "TimeTree n'a pas d'accès officiel : le serveur se connecte comme le site web,"
            );
            println!(
                "en lecture seule. Utilise un compte DÉDIÉ à la maison, invité dans ton agenda"
            );
            println!("partagé, jamais ton compte personnel. Le mot de passe est gardé chiffré.");
            println!();
            let email = ask_line("e-mail du compte TimeTree de la maison : ")?;
            if !(3..=254).contains(&email.len())
                || !email.contains('@')
                || email.chars().any(char::is_control)
            {
                return Err("adresse e-mail invalide".into());
            }
            let password = ask_secret("mot de passe : ")?;
            let account = ha_storage::TimeTreeAccount { email, password };
            let client = timetree_client(account.clone())?;
            let calendars = client.calendars().await?;
            store.save_timetree_account(&account).await?;
            println!("associé !");
            print_timetree_calendars(&calendars);
            println!("puis relance « ha-server ».");
            Ok(())
        }
        ["status"] => {
            let Some(account) = store.timetree_account().await? else {
                println!("aucun compte TimeTree associé (ha-server timetree pair)");
                return Ok(());
            };
            println!("compte : {}", account.email);
            let calendars = timetree_client(account)?.calendars().await?;
            print_timetree_calendars(&calendars);
            Ok(())
        }
        ["forget"] => {
            if store.forget_timetree_account().await? {
                println!("compte TimeTree oublié (pense à changer son mot de passe s'il a fuité)");
            } else {
                println!("aucun compte TimeTree associé");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// `iglou pair|status|forget` : la liste de courses de l'appli Iglou
/// (étape 81).
pub(crate) async fn iglou_command(args: &[&str]) -> Result<(), Box<dyn Error>> {
    let store = crate::open_store().await?;
    let result = iglou_inner(args, &store).await;
    if result.is_ok() {
        audit_cli(&store, "iglou", args).await;
    }
    store.close().await;
    result
}

fn print_iglou_list(items: &[ha_iglou::list::Item]) {
    let left = items.iter().filter(|i| !i.done).count();
    println!("liste de courses : {left} article(s) à acheter");
    for item in items.iter().take(30) {
        println!(
            "  {} {} ({}, {})",
            if item.done { "[x]" } else { "[ ]" },
            item.summary,
            item.quantity,
            item.group
        );
    }
    if items.len() > 30 {
        println!("  … et {} de plus", items.len() - 30);
    }
}

async fn iglou_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    match args {
        ["pair"] => {
            println!("Iglou : la maison lit et complète la liste de courses de l'appli.");
            println!("Utilise un compte DÉDIÉ à la maison (créé dans l'appli), jamais ton compte");
            println!("personnel. Le mot de passe est gardé chiffré.");
            println!();
            println!("Le projet et la clé web sont dans la configuration Firebase de l'appli");
            println!("(google-services.json : « project_id » et « current_key »).");
            let project_id = ask_line("projet Firebase : ")?;
            if !ha_iglou::client::is_project_id(&project_id) {
                return Err("projet invalide (minuscules, chiffres et tirets)".into());
            }
            let api_key = ask_line("clé web Firebase : ")?;
            if !ha_iglou::client::is_api_key(&api_key) {
                return Err("clé web invalide".into());
            }
            let email = ask_line("e-mail du compte Iglou de la maison : ")?;
            if !(3..=254).contains(&email.len())
                || !email.contains('@')
                || email.chars().any(char::is_control)
            {
                return Err("adresse e-mail invalide".into());
            }
            let password = ask_secret("mot de passe : ")?;
            let mut account = ha_storage::IglouAccount {
                project_id,
                api_key,
                email,
                password,
                maison_id: String::new(),
            };
            let client = crate::iglou_client(&account)?;
            let maisons = client.maisons().await?;
            let maison = match maisons.as_slice() {
                [] => {
                    println!("Ce compte n'est membre d'aucune maison Iglou.");
                    println!(
                        "Dans l'appli, sur ton téléphone : Réglages → Code d'invitation, puis recopie-le."
                    );
                    let code = ask_line("code d'invitation : ")?;
                    client.join(&code, "Maison").await?
                }
                [one] => one.clone(),
                many => {
                    for (index, maison) in many.iter().enumerate() {
                        println!("  {}. {}", index + 1, maison.nom);
                    }
                    let choice = ask_line("laquelle (numéro) : ")?;
                    choice
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| many.get(n.checked_sub(1)?))
                        .cloned()
                        .ok_or("choix invalide")?
                }
            };
            account.maison_id = maison.id.clone();
            let items = client.shopping_list(&maison.id).await?;
            store.save_iglou_account(&account).await?;
            println!("associé à la maison « {} » !", maison.nom);
            // Ce qui restait sur la liste locale passe dans Iglou.
            let local = store.todo_items(ha_iglou::ENTITY).await?;
            let mut copied = 0;
            for item in local.iter().filter(|i| !i.done) {
                client.add(&maison.id, &item.summary).await?;
                store.remove_todo_item(ha_iglou::ENTITY, item.id).await?;
                copied += 1;
            }
            if copied > 0 {
                println!("{copied} article(s) de l'ancienne liste locale ajouté(s) dans Iglou");
            }
            print_iglou_list(&items);
            println!("puis relance « ha-server » : todo.courses devient la liste d'Iglou.");
            Ok(())
        }
        ["status"] => {
            let Some(account) = store.iglou_account().await? else {
                println!("aucun compte Iglou associé (ha-server iglou pair)");
                return Ok(());
            };
            println!("compte : {} (projet {})", account.email, account.project_id);
            let client = crate::iglou_client(&account)?;
            let maisons = client.maisons().await?;
            match maisons.iter().find(|m| m.id == account.maison_id) {
                Some(maison) => println!("maison : {}", maison.nom),
                None => return Err("le compte n'est plus membre de sa maison Iglou : refais « ha-server iglou pair »".into()),
            }
            print_iglou_list(&client.shopping_list(&account.maison_id).await?);
            Ok(())
        }
        ["forget"] => {
            if store.forget_iglou_account().await? {
                println!(
                    "compte Iglou oublié : la liste de courses locale reprend au prochain démarrage."
                );
                println!("Pour couper l'accès, retire aussi le membre « Maison » dans l'appli.");
            } else {
                println!("aucun compte Iglou associé");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::is_microsoft_mailbox;

    #[test]
    fn microsoft_mailboxes_are_spotted() {
        assert!(is_microsoft_mailbox("imap.gmail.com", "david@hotmail.fr"));
        assert!(is_microsoft_mailbox("imap.gmail.com", "David@Outlook.com"));
        assert!(is_microsoft_mailbox(
            "outlook.office365.com",
            "a@exemple.fr"
        ));
        assert!(!is_microsoft_mailbox("imap.gmail.com", "maison@gmail.com"));
        assert!(!is_microsoft_mailbox("imap.gmail.com", "a@livebox.fr"));
    }
}
