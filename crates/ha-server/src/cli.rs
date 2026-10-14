//! Commandes `ha-server user …` : gestion des comptes en ligne de commande.
//!
//! Il n'y a volontairement pas de route d'API pour créer le premier compte :
//! quelqu'un qui joint le serveur par le réseau ne doit pas pouvoir devenir
//! administrateur. Il faut avoir accès à la machine elle-même.

use std::error::Error;
use std::io::{BufRead, IsTerminal};

use ha_auth::{Auth, MIN_PASSWORD_LEN};
use ha_storage::Store;
use zeroize::Zeroizing;

pub const USAGE: &str = "\
utilisation :
  ha-server                       démarre le serveur
  ha-server user add <nom>        crée un compte
  ha-server user passwd <nom>     change un mot de passe
  ha-server user delete <nom>     supprime un compte
  ha-server user list             liste les comptes";

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
