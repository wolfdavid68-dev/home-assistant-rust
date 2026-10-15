//! L'éditeur : les automatisations créées et modifiées depuis le tableau de
//! bord.
//!
//! Il y a deux sources d'automatisations :
//! - le fichier `automations.yaml`, **en lecture seule** : le serveur ne le
//!   modifie jamais (avec Docker, il est même monté en lecture seule) ;
//! - la base de données, pour celles de l'éditeur, avec leurs versions
//!   précédentes.
//!
//! Chaque enregistrement passe par **les mêmes vérifications** que le
//! fichier, puis le moteur est rechargé, sans redémarrer le serveur.

use std::collections::HashMap;
use std::time::SystemTime;

use ha_storage::Store;
use tokio::sync::Mutex;

use crate::config::{Automation, AutomationConfig, MAX_AUTOMATIONS, check_all, uses_sun};
use crate::engine::Reloader;
use crate::error::AutomationError;

/// D'où vient une automatisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `automations.yaml` : lecture seule.
    File,
    /// L'éditeur du tableau de bord.
    Editor,
}

/// Une automatisation, telle que l'éditeur l'affiche.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub source: Source,
    /// `None` si la configuration rangée dans la base est illisible.
    pub config: Option<AutomationConfig>,
    /// Pourquoi elle ne tourne pas, si elle est invalide (par exemple : elle
    /// utilise le soleil, mais la position de la maison a été retirée).
    pub error: Option<String>,
    pub updated_at: Option<SystemTime>,
    pub updated_by: Option<String>,
}

/// Une version précédente. `config` vaut `None` pour une suppression.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub version: i64,
    pub config: Option<AutomationConfig>,
    pub changed_at: SystemTime,
    pub changed_by: String,
}

/// Blocs placés au plus dans le schéma d'une automatisation.
const MAX_LAYOUT_NODES: usize = 400;
/// Les coordonnées restent raisonnables : le schéma n'a pas à être immense.
const MAX_COORDINATE: f64 = 100_000.0;

/// Vérifie la place des blocs (`{"1 › alors › 2": [x, y], …}`) et la
/// renvoie sous une forme propre. Le contenu vient du navigateur : rien
/// d'autre que des positions n'est gardé.
fn check_layout(layout: &serde_json::Value) -> Result<String, String> {
    let serde_json::Value::Object(map) = layout else {
        return Err("schéma : un objet { bloc: [x, y] } attendu".into());
    };
    if map.len() > MAX_LAYOUT_NODES {
        return Err(format!("schéma : {MAX_LAYOUT_NODES} blocs au plus"));
    }
    let mut clean = serde_json::Map::new();
    for (key, value) in map {
        if key.is_empty() || key.chars().count() > 120 || key.chars().any(char::is_control) {
            return Err("schéma : nom de bloc invalide".into());
        }
        let point = value
            .as_array()
            .filter(|point| point.len() == 2)
            .and_then(|point| Some((point[0].as_f64()?, point[1].as_f64()?)))
            .filter(|(x, y)| {
                [x, y]
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
            });
        let Some((x, y)) = point else {
            return Err(format!("schéma : position de « {key} » invalide"));
        };
        // Au pixel près : inutile de garder des décimales.
        clean.insert(key.clone(), serde_json::json!([x.round(), y.round()]));
    }
    Ok(serde_json::Value::Object(clean).to_string())
}

/// Résultat d'un enregistrement.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub automation: Automation,
    /// `true` pour une création, `false` pour une modification.
    pub created: bool,
}

/// L'éditeur d'automatisations.
#[derive(Debug)]
pub struct Editor {
    store: Store,
    /// Les automatisations du fichier (déjà vérifiées au démarrage).
    file: Vec<AutomationConfig>,
    /// La position de la maison est-elle connue ? (Nécessaire pour le soleil.)
    has_location: bool,
    reloader: Option<Reloader>,
    /// Un seul enregistrement à la fois : deux modifications simultanées ne
    /// peuvent pas se mélanger, ni dépasser ensemble la limite du nombre
    /// d'automatisations.
    lock: Mutex<()>,
}

impl Editor {
    #[must_use]
    pub fn new(store: Store, file: Vec<AutomationConfig>, has_location: bool) -> Self {
        Self {
            store,
            file,
            has_location,
            reloader: None,
            lock: Mutex::new(()),
        }
    }

    /// Après chaque modification, le moteur sera rechargé par `reloader`.
    #[must_use]
    pub fn with_reloader(mut self, reloader: Reloader) -> Self {
        self.reloader = Some(reloader);
        self
    }

    /// Vérifie une automatisation de l'éditeur, sans l'enregistrer.
    pub fn check(&self, config: &AutomationConfig) -> Result<Automation, AutomationError> {
        let automation = config.check()?;
        let invalid = |reason: &str| AutomationError::Invalid {
            id: config.id().to_owned(),
            reason: reason.to_owned(),
        };
        if self.file.iter().any(|file| file.id() == config.id()) {
            return Err(invalid(
                "cet identifiant est déjà utilisé dans automations.yaml",
            ));
        }
        if !self.has_location && uses_sun(std::slice::from_ref(&automation)) {
            return Err(invalid(
                "elle utilise le soleil : indique d'abord la position de la maison \
                 (HA_LATITUDE et HA_LONGITUDE)",
            ));
        }
        Ok(automation)
    }

    /// Toutes les automatisations : celles du fichier, puis celles de l'éditeur.
    pub async fn entries(&self) -> Result<Vec<Entry>, AutomationError> {
        let mut entries: Vec<Entry> = self
            .file
            .iter()
            .map(|config| Entry {
                id: config.id().to_owned(),
                source: Source::File,
                config: Some(config.clone()),
                error: None,
                updated_at: None,
                updated_by: None,
            })
            .collect();
        entries.extend(self.stored().await?.into_iter().map(|(entry, _)| entry));
        Ok(entries)
    }

    /// Les automatisations à faire tourner : celles du fichier, et celles de
    /// l'éditeur qui sont valides et activées.
    ///
    /// Une automatisation de l'éditeur devenue invalide est **mise de côté**
    /// (avec un message dans les logs et dans le tableau de bord) au lieu
    /// d'empêcher le démarrage : sinon, impossible de la corriger depuis le
    /// tableau de bord.
    pub async fn active(&self) -> Result<Vec<Automation>, AutomationError> {
        let mut automations = check_all(&self.file)?;
        for (entry, automation) in self.stored().await? {
            if let Some(error) = &entry.error {
                tracing::error!("automatisation « {} » ignorée : {error}", entry.id);
            }
            let enabled = entry.config.as_ref().is_some_and(AutomationConfig::enabled);
            if let Some(automation) = automation.filter(|_| enabled) {
                automations.push(automation);
            }
        }
        Ok(automations)
    }

    /// Les automatisations de la base, relues et vérifiées.
    async fn stored(&self) -> Result<Vec<(Entry, Option<Automation>)>, AutomationError> {
        let rows = self.store.automations().await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let config = serde_json::from_str(&row.config)
                    .map_err(|error| AutomationError::Yaml(error.to_string()))
                    .and_then(AutomationConfig::from_json);
                let checked = config.as_ref().map_err(ToString::to_string).and_then(|c| {
                    if c.id() == row.id {
                        self.check(c).map_err(|error| error.to_string())
                    } else {
                        Err("identifiant incohérent".to_owned())
                    }
                });
                let (automation, error) = match checked {
                    Ok(automation) => (Some(automation), None),
                    Err(error) => (None, Some(error)),
                };
                let entry = Entry {
                    id: row.id,
                    source: Source::Editor,
                    config: config.ok(),
                    error,
                    updated_at: Some(row.updated_at),
                    updated_by: Some(row.updated_by),
                };
                (entry, automation)
            })
            .collect())
    }

    /// Crée ou modifie une automatisation, puis recharge le moteur.
    pub async fn save(
        &self,
        config: &AutomationConfig,
        actor: &str,
    ) -> Result<Saved, AutomationError> {
        let _guard = self.lock.lock().await;
        let automation = self.check(config)?;
        let stored = self.store.automations().await?;
        let exists = stored.iter().any(|row| row.id == config.id());
        if !exists && self.file.len() + stored.len() >= MAX_AUTOMATIONS {
            return Err(AutomationError::TooMany(MAX_AUTOMATIONS));
        }
        self.store
            .save_automation(config.id(), &config.to_json().to_string(), actor)
            .await?;
        self.reload().await?;
        Ok(Saved {
            automation,
            created: !exists,
        })
    }

    /// Supprime une automatisation de l'éditeur, puis recharge le moteur.
    /// Renvoie `false` si elle n'existait pas.
    pub async fn delete(&self, id: &str, actor: &str) -> Result<bool, AutomationError> {
        let _guard = self.lock.lock().await;
        if self.file.iter().any(|file| file.id() == id) {
            return Err(AutomationError::Invalid {
                id: id.to_owned(),
                reason: "elle est définie dans automations.yaml : modifie le fichier".into(),
            });
        }
        let deleted = self.store.delete_automation(id, actor).await?;
        if deleted {
            self.reload().await?;
        }
        Ok(deleted)
    }

    /// La place des blocs dans le schéma, pour chaque automatisation qui en a
    /// une. Une valeur illisible est ignorée (le schéma se replace tout seul).
    pub async fn layouts(&self) -> Result<HashMap<String, serde_json::Value>, AutomationError> {
        Ok(self
            .store
            .automation_layouts()
            .await?
            .into_iter()
            .filter_map(|(id, text)| Some((id, serde_json::from_str(&text).ok()?)))
            .collect())
    }

    /// Enregistre la place des blocs d'une automatisation (du fichier ou de
    /// l'éditeur). Rien ne change dans ce qu'elle fait : pas de nouvelle
    /// version, pas de rechargement.
    pub async fn save_layout(
        &self,
        id: &str,
        layout: &serde_json::Value,
    ) -> Result<(), AutomationError> {
        let invalid = |reason: String| AutomationError::Invalid {
            id: id.to_owned(),
            reason,
        };
        if !self.entries().await?.iter().any(|entry| entry.id == id) {
            return Err(invalid("automatisation inconnue".into()));
        }
        let layout = check_layout(layout).map_err(invalid)?;
        self.store.save_automation_layout(id, &layout).await?;
        Ok(())
    }

    /// Les versions d'une automatisation de l'éditeur, de la plus récente à
    /// la plus ancienne.
    pub async fn versions(&self, id: &str) -> Result<Vec<Version>, AutomationError> {
        let versions = self.store.automation_versions(id).await?;
        Ok(versions
            .into_iter()
            .map(|version| Version {
                version: version.version,
                config: version.config.and_then(|text| {
                    AutomationConfig::from_json(serde_json::from_str(&text).ok()?).ok()
                }),
                changed_at: version.changed_at,
                changed_by: version.changed_by,
            })
            .collect())
    }

    async fn reload(&self) -> Result<(), AutomationError> {
        if let Some(reloader) = &self.reloader {
            reloader.reload(self.active().await?);
        }
        Ok(())
    }
}
