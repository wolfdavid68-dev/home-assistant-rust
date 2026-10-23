//! Parler à une enceinte Sonos : de l'UPnP, c'est-à-dire des requêtes SOAP
//! (du XML en HTTP) sur le port 1400, en local, sans compte ni cloud.
//!
//! Chaque enceinte a trois « services » qui nous servent :
//! - `AVTransport` : lecture, pause, morceau suivant, ce qui est chargé ;
//! - `RenderingControl` : volume et sourdine ;
//! - `ZoneGroupTopology` : la liste de toutes les enceintes et leurs groupes ;
//! - `ContentDirectory` : les favoris (étape 91).

use std::time::Duration;

use crate::favorites::{self, Favorite};
use crate::xml;

/// Port de l'API locale des Sonos.
pub const PORT: u16 = 1400;
/// Une enceinte qui ne répond pas en 3 s est considérée injoignable.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
/// Ajouter une playlist à la file d'attente : l'enceinte va la chercher
/// chez le service de musique, c'est plus long.
const QUEUE_TIMEOUT: Duration = Duration::from_secs(20);
/// Taille maximale d'une réponse (la topologie d'une grande maison fait
/// quelques dizaines de Ko).
const MAX_RESPONSE: usize = 512 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SonosError {
    #[error("enceinte injoignable ({0})")]
    Unreachable(String),
    /// Une erreur UPnP : le code dit pourquoi (701 : « pas possible
    /// maintenant », 714 : « format non pris en charge »…).
    #[error("l'enceinte a refusé (erreur UPnP {0})")]
    Refused(u32),
    #[error("réponse illisible de l'enceinte ({0})")]
    Unreadable(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    AvTransport,
    RenderingControl,
    ZoneGroupTopology,
    ContentDirectory,
}

impl Service {
    fn path(self) -> &'static str {
        match self {
            Self::AvTransport => "/MediaRenderer/AVTransport/Control",
            Self::RenderingControl => "/MediaRenderer/RenderingControl/Control",
            Self::ZoneGroupTopology => "/ZoneGroupTopology/Control",
            Self::ContentDirectory => "/MediaServer/ContentDirectory/Control",
        }
    }

    fn urn(self) -> &'static str {
        match self {
            Self::AvTransport => "urn:schemas-upnp-org:service:AVTransport:1",
            Self::RenderingControl => "urn:schemas-upnp-org:service:RenderingControl:1",
            Self::ZoneGroupTopology => "urn:schemas-upnp-org:service:ZoneGroupTopology:1",
            Self::ContentDirectory => "urn:schemas-upnp-org:service:ContentDirectory:1",
        }
    }
}

/// Ce qui est en cours de lecture (`GetPositionInfo`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Position {
    /// Numéro du morceau dans la file d'attente (à partir de 1).
    pub track: u32,
    pub track_uri: String,
    /// Les métadonnées DIDL du morceau (XML).
    pub metadata: String,
    /// Où on en est dans le morceau : `0:01:23`.
    pub rel_time: String,
    /// La durée du morceau : `0:03:45` (vide ou `NOT_IMPLEMENTED` pour
    /// une radio).
    pub duration: String,
}

/// Ce qui est chargé (`GetMediaInfo`) : la file d'attente, une radio, la TV…
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Media {
    pub uri: String,
    pub metadata: String,
}

/// Le client, partagé par toutes les enceintes.
#[derive(Debug, Clone)]
pub struct Api {
    http: reqwest::Client,
}

impl Api {
    pub fn new() -> Result<Self, ha_tls::TlsError> {
        // HTTP simple : les Sonos ne parlent pas HTTPS sur ce port.
        Ok(Self {
            http: ha_tls::web_client(false)?,
        })
    }

    /// Le client HTTP, pour aller chercher les pochettes (voir
    /// [`crate::art`]).
    #[must_use]
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Une action SOAP : les arguments, puis la réponse XML.
    pub async fn call(
        &self,
        host: &str,
        service: Service,
        action: &str,
        args: &[(&str, &str)],
    ) -> Result<String, SonosError> {
        self.call_within(host, service, action, args, CALL_TIMEOUT)
            .await
    }

    async fn call_within(
        &self,
        host: &str,
        service: Service,
        action: &str,
        args: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<String, SonosError> {
        let mut arguments = String::new();
        for (name, value) in args {
            arguments.push('<');
            arguments.push_str(name);
            arguments.push('>');
            arguments.push_str(&xml::escape(value));
            arguments.push_str("</");
            arguments.push_str(name);
            arguments.push('>');
        }
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
             <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
             s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
             <u:{action} xmlns:u=\"{urn}\">{arguments}</u:{action}></s:Body></s:Envelope>",
            urn = service.urn()
        );
        let url = format!("http://{host}{}", service.path());
        let response = self
            .http
            .post(&url)
            .timeout(timeout)
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .header("SOAPACTION", format!("\"{}#{action}\"", service.urn()))
            .body(body)
            .send()
            .await
            .map_err(|error| SonosError::Unreachable(error.without_url().to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| SonosError::Unreachable(error.without_url().to_string()))?;
        if bytes.len() > MAX_RESPONSE {
            return Err(SonosError::Unreadable("réponse trop longue"));
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if !status.is_success() {
            let code = xml::element_text(&text, "errorCode")
                .and_then(|code| code.trim().parse().ok())
                .unwrap_or(u32::from(status.as_u16()));
            return Err(SonosError::Refused(code));
        }
        Ok(text)
    }

    async fn av(
        &self,
        host: &str,
        action: &str,
        args: &[(&str, &str)],
    ) -> Result<String, SonosError> {
        let mut all = vec![("InstanceID", "0")];
        all.extend_from_slice(args);
        self.call(host, Service::AvTransport, action, &all).await
    }

    async fn rendering(
        &self,
        host: &str,
        action: &str,
        args: &[(&str, &str)],
    ) -> Result<String, SonosError> {
        let mut all = vec![("InstanceID", "0"), ("Channel", "Master")];
        all.extend_from_slice(args);
        self.call(host, Service::RenderingControl, action, &all)
            .await
    }

    /// `PLAYING`, `PAUSED_PLAYBACK`, `STOPPED`, `TRANSITIONING`…
    pub async fn transport_state(&self, host: &str) -> Result<String, SonosError> {
        let answer = self.av(host, "GetTransportInfo", &[]).await?;
        xml::element_text(&answer, "CurrentTransportState")
            .ok_or(SonosError::Unreadable("état de lecture absent"))
    }

    pub async fn position(&self, host: &str) -> Result<Position, SonosError> {
        let answer = self.av(host, "GetPositionInfo", &[]).await?;
        let field = |name| xml::element_text(&answer, name).unwrap_or_default();
        Ok(Position {
            track: field("Track").trim().parse().unwrap_or(0),
            track_uri: field("TrackURI"),
            metadata: field("TrackMetaData"),
            rel_time: field("RelTime"),
            duration: field("TrackDuration"),
        })
    }

    pub async fn media(&self, host: &str) -> Result<Media, SonosError> {
        let answer = self.av(host, "GetMediaInfo", &[]).await?;
        Ok(Media {
            uri: xml::element_text(&answer, "CurrentURI").unwrap_or_default(),
            metadata: xml::element_text(&answer, "CurrentURIMetaData").unwrap_or_default(),
        })
    }

    pub async fn volume(&self, host: &str) -> Result<u8, SonosError> {
        let answer = self.rendering(host, "GetVolume", &[]).await?;
        xml::element_text(&answer, "CurrentVolume")
            .and_then(|v| v.trim().parse::<u8>().ok())
            .map(|v| v.min(100))
            .ok_or(SonosError::Unreadable("volume absent"))
    }

    pub async fn muted(&self, host: &str) -> Result<bool, SonosError> {
        let answer = self.rendering(host, "GetMute", &[]).await?;
        xml::element_text(&answer, "CurrentMute")
            .map(|v| v.trim() == "1")
            .ok_or(SonosError::Unreadable("sourdine absente"))
    }

    pub async fn set_volume(&self, host: &str, volume: u8) -> Result<(), SonosError> {
        let volume = volume.min(100).to_string();
        self.rendering(host, "SetVolume", &[("DesiredVolume", &volume)])
            .await
            .map(drop)
    }

    pub async fn set_mute(&self, host: &str, muted: bool) -> Result<(), SonosError> {
        let value = if muted { "1" } else { "0" };
        self.rendering(host, "SetMute", &[("DesiredMute", value)])
            .await
            .map(drop)
    }

    pub async fn play(&self, host: &str) -> Result<(), SonosError> {
        self.av(host, "Play", &[("Speed", "1")]).await.map(drop)
    }

    pub async fn pause(&self, host: &str) -> Result<(), SonosError> {
        self.av(host, "Pause", &[]).await.map(drop)
    }

    pub async fn stop(&self, host: &str) -> Result<(), SonosError> {
        self.av(host, "Stop", &[]).await.map(drop)
    }

    pub async fn next(&self, host: &str) -> Result<(), SonosError> {
        self.av(host, "Next", &[]).await.map(drop)
    }

    pub async fn previous(&self, host: &str) -> Result<(), SonosError> {
        self.av(host, "Previous", &[]).await.map(drop)
    }

    /// Charge une adresse (sans la lancer).
    pub async fn set_uri(&self, host: &str, uri: &str, metadata: &str) -> Result<(), SonosError> {
        self.av(
            host,
            "SetAVTransportURI",
            &[("CurrentURI", uri), ("CurrentURIMetaData", metadata)],
        )
        .await
        .map(drop)
    }

    /// Va au morceau `TRACK_NR` ou au temps `REL_TIME` (`0:01:23`).
    pub async fn seek(&self, host: &str, unit: &str, target: &str) -> Result<(), SonosError> {
        self.av(host, "Seek", &[("Unit", unit), ("Target", target)])
            .await
            .map(drop)
    }

    /// Le mode de lecture : `NORMAL`, `SHUFFLE`, `REPEAT_ALL`… (étape 92).
    pub async fn play_mode(&self, host: &str) -> Result<String, SonosError> {
        let answer = self.av(host, "GetTransportSettings", &[]).await?;
        xml::element_text(&answer, "PlayMode")
            .map(|mode| mode.trim().to_owned())
            .ok_or(SonosError::Unreadable("mode de lecture absent"))
    }

    pub async fn set_play_mode(&self, host: &str, mode: &str) -> Result<(), SonosError> {
        self.av(host, "SetPlayMode", &[("NewPlayMode", mode)])
            .await
            .map(drop)
    }

    /// Les favoris de la maison (étape 91).
    pub async fn favorites(&self, host: &str) -> Result<Vec<Favorite>, SonosError> {
        let count = favorites::MAX.to_string();
        let answer = self
            .call(
                host,
                Service::ContentDirectory,
                "Browse",
                &[
                    ("ObjectID", "FV:2"),
                    ("BrowseFlag", "BrowseDirectChildren"),
                    ("Filter", "*"),
                    ("StartingIndex", "0"),
                    ("RequestedCount", &count),
                    ("SortCriteria", ""),
                ],
            )
            .await?;
        let didl = xml::element_text(&answer, "Result")
            .ok_or(SonosError::Unreadable("favoris absents"))?;
        Ok(favorites::parse(&didl))
    }

    /// Vide la file d'attente.
    pub async fn clear_queue(&self, host: &str) -> Result<(), SonosError> {
        self.av(host, "RemoveAllTracksFromQueue", &[])
            .await
            .map(drop)
    }

    /// Ajoute une playlist, un album ou un morceau à la fin de la file.
    pub async fn add_to_queue(
        &self,
        host: &str,
        uri: &str,
        metadata: &str,
    ) -> Result<(), SonosError> {
        self.call_within(
            host,
            Service::AvTransport,
            "AddURIToQueue",
            &[
                ("InstanceID", "0"),
                ("EnqueuedURI", uri),
                ("EnqueuedURIMetaData", metadata),
                ("DesiredFirstTrackNumberEnqueued", "0"),
                ("EnqueueAsNext", "0"),
            ],
            QUEUE_TIMEOUT,
        )
        .await
        .map(drop)
    }

    /// La liste des groupes et de leurs enceintes (XML).
    pub async fn zone_group_state(&self, host: &str) -> Result<String, SonosError> {
        let answer = self
            .call(host, Service::ZoneGroupTopology, "GetZoneGroupState", &[])
            .await?;
        xml::element_text(&answer, "ZoneGroupState")
            .ok_or(SonosError::Unreadable("groupes absents"))
    }
}
