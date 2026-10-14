//! Erreurs de l'intégration Freebox.

#[derive(Debug, thiserror::Error)]
pub enum FreeboxError {
    #[error("connexion impossible : {}", chain(.0))]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Tls(#[from] ha_tls::TlsError),

    #[error("adresse de la Freebox invalide : {0:?} (exemple : mafreebox.freebox.fr)")]
    InvalidHost(String),

    #[error("autorisation refusée sur l'écran de la Freebox")]
    Denied,

    #[error("pas de réponse sur l'écran de la Freebox à temps : recommence")]
    Timeout,

    #[error("jeton refusé par la Freebox (application révoquée dans Freebox OS ?)")]
    Unauthorized,

    #[error(
        "droits insuffisants : autorise-les dans Freebox OS (Paramètres > Gestion des accès > Applications)"
    )]
    InsufficientRights,

    #[error("la Freebox a répondu une erreur : {0}")]
    Api(String),

    #[error("réponse inattendue : {0}")]
    Unexpected(String),

    #[error("réponse trop grosse")]
    TooLarge,

    #[error("adresse MAC invalide : {0:?} (exemple : AA:BB:CC:DD:EE:FF)")]
    InvalidMac(String),

    #[error("nom invalide : {0:?} (minuscules, chiffres et « _ », par exemple telephone_david)")]
    InvalidName(String),

    #[error("erreur de base de données : {0}")]
    Database(#[from] sqlx::Error),

    #[error("erreur de stockage : {0}")]
    Storage(#[from] ha_storage::StorageError),
}

/// Le message d'une erreur et de toutes ses causes : « erreur d'envoi :
/// certificat inattendu… » plutôt que seulement « erreur d'envoi ».
fn chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(" : ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}
