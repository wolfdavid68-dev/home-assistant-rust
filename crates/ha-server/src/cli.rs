//! Commandes `ha-server user …` : gestion des comptes en ligne de commande.
//!
//! Il n'y a volontairement pas de route d'API pour créer le premier compte :
//! quelqu'un qui joint le serveur par le réseau ne doit pas pouvoir devenir
//! administrateur. Il faut avoir accès à la machine elle-même.

use std::error::Error;
use std::io::{BufRead, IsTerminal};

use ha_auth::{Auth, MIN_PASSWORD_LEN};
use ha_hue::{pairing, tls};
use ha_storage::Store;
use zeroize::Zeroizing;

pub const USAGE: &str = "\
utilisation :
  ha-server                       démarre le serveur
  ha-server user add <nom>        crée un compte
  ha-server user passwd <nom>     change un mot de passe
  ha-server user delete <nom>     supprime un compte
  ha-server user list             liste les comptes
  ha-server hue pair <adresse>    associe le pont Philips Hue
  ha-server hue status            affiche le pont associé
  ha-server hue forget            oublie le pont
  ha-server automations check     vérifie le fichier d'automatisations";

pub async fn user_command(args: &[&str], db_path: &str) -> Result<(), Box<dyn Error>> {
    let store = Store::open(db_path).await?;
    let auth = Auth::new(&store).await?;

    let result = match args {
        ["add", name] => {
            let password = ask_new_password()?;
            auth.create_user(name, &password).await?;
            println!("compte « {name} » créé");
            Ok(())
        }
        ["passwd", name] => {
            let password = ask_new_password()?;
            if auth.set_password(name, &password).await? {
                println!("mot de passe de « {name} » changé ; toutes ses sessions sont fermées");
                Ok(())
            } else {
                Err(format!("le compte « {name} » n'existe pas").into())
            }
        }
        ["delete", name] => {
            if auth.delete_user(name).await? {
                println!("compte « {name} » supprimé");
                Ok(())
            } else {
                Err(format!("le compte « {name} » n'existe pas").into())
            }
        }
        ["list"] => {
            for user in auth.list_users().await? {
                println!("{}", user.username);
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    };
    store.close().await;
    result
}

/// Demande un nouveau mot de passe deux fois, sans l'afficher à l'écran.
///
/// Si l'entrée n'est pas un terminal (script, Docker…), le mot de passe est
/// lu sur une seule ligne de l'entrée standard.
fn ask_new_password() -> Result<Zeroizing<String>, Box<dyn Error>> {
    if !std::io::stdin().is_terminal() {
        let mut line = Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line)?;
        let password = Zeroizing::new(line.trim_end_matches(['\r', '\n']).to_owned());
        return Ok(password);
    }

    println!("au moins {MIN_PASSWORD_LEN} caractères ; une phrase de passe est idéale");
    let first = Zeroizing::new(rpassword::prompt_password("mot de passe : ")?);
    let second = Zeroizing::new(rpassword::prompt_password("encore une fois : ")?);
    if first != second {
        return Err("les deux mots de passe sont différents".into());
    }
    Ok(first)
}

pub async fn hue_command(args: &[&str], db_path: &str) -> Result<(), Box<dyn Error>> {
    let store = Store::open(db_path).await?;
    let result = hue_inner(args, &store).await;
    store.close().await;
    result
}

async fn hue_inner(args: &[&str], store: &Store) -> Result<(), Box<dyn Error>> {
    match args {
        ["pair", host] => {
            let (bridge_id, fingerprint) = pairing::identify(host).await?;
            println!("pont Hue trouvé : {bridge_id}");
            println!("empreinte de son certificat :");
            println!("  {}", tls::format_fingerprint(&fingerprint));
            println!("seul ce certificat sera accepté à l'avenir.");
            println!();
            println!(
                "appuie maintenant sur le gros bouton rond du pont (tu as {} secondes)…",
                pairing::LINK_BUTTON_TIMEOUT.as_secs()
            );
            let paired = pairing::request_key(host, bridge_id, fingerprint).await?;
            pairing::save(store, &paired).await?;
            println!("associé ! relance « ha-server » pour piloter tes lampes.");
            Ok(())
        }
        ["status"] => {
            match pairing::load(store).await? {
                Some(paired) => {
                    println!("pont {} à l'adresse {}", paired.bridge_id, paired.host);
                    println!(
                        "certificat : {}",
                        tls::format_fingerprint(&paired.fingerprint)
                    );
                }
                None => println!("aucun pont associé (ha-server hue pair <adresse>)"),
            }
            Ok(())
        }
        ["forget"] => {
            if pairing::forget(store).await? {
                println!("pont oublié.");
                println!("retire aussi « ha_rust » des applications autorisées dans l'appli Hue");
                println!("(Paramètres > Ponts Hue > i > Applications) pour révoquer la clé.");
            } else {
                println!("aucun pont associé.");
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

/// Vérifie le fichier d'automatisations sans démarrer le serveur.
pub fn check_automations(path: &std::path::Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        println!("{} n'existe pas : aucune automatisation.", path.display());
        return Ok(());
    }
    let automations = ha_automation::load(path)?;
    println!(
        "{} : {} automatisation(s) valide(s)",
        path.display(),
        automations.len()
    );
    for automation in &automations {
        println!(
            "  - {} ({} déclencheur(s), {} condition(s), {} action(s))",
            automation.alias,
            automation.triggers.len(),
            automation.conditions.len(),
            automation.actions.len()
        );
    }
    Ok(())
}
