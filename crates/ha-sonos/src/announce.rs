//! Une annonce sur un groupe d'enceintes, sans perdre la musique.
//!
//! 1. **On note** ce qui était chargé (la file d'attente, une radio, la
//!    TV), le morceau et l'endroit du morceau, si ça jouait, et le volume de
//!    chaque enceinte ;
//! 2. **on annonce** : volume de l'annonce, son chargé, lecture ;
//! 3. **on attend** la fin du son ;
//! 4. **on remet** tout comme avant : la même musique au même endroit, le
//!    même volume, et la lecture si ça jouait.
//!
//! Si quelqu'un lance autre chose pendant l'annonce, on ne remet pas
//! l'ancienne musique par-dessus : seulement les volumes.
//!
//! Limite (comme dans Home Assistant) : ce qui est joué par Spotify Connect
//! ou AirPlay depuis un téléphone ne peut pas être repris ; la musique
//! s'arrête après l'annonce, et le téléphone la relance.

use std::time::Duration;

use tracing::{debug, info, warn};

use crate::api::{Api, Media, Position, SonosError};
use crate::xml;

/// Un groupe à faire parler : l'adresse de son coordinateur (qui lit), et
/// celles de toutes ses enceintes (dont on règle le volume).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub coordinator: String,
    pub members: Vec<String>,
}

/// Les délais (raccourcis par les tests).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Entre deux lectures de l'état pendant l'annonce.
    pub poll: Duration,
    /// Le temps laissé à l'enceinte pour télécharger le son et démarrer.
    pub start: Duration,
    /// La marge après la durée du son.
    pub margin: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(500),
            start: Duration::from_secs(10),
            margin: Duration::from_secs(10),
        }
    }
}

/// Ce qu'on a noté avant l'annonce.
#[derive(Debug, Clone)]
pub struct Snapshot {
    transport: String,
    media: Media,
    position: Position,
    volumes: Vec<(String, u8, bool)>,
}

impl Snapshot {
    fn was_playing(&self) -> bool {
        matches!(self.transport.as_str(), "PLAYING" | "TRANSITIONING")
    }
}

/// Les métadonnées DIDL de l'annonce : le titre affiché dans l'appli Sonos.
fn metadata(url: &str) -> String {
    format!(
        "<DIDL-Lite xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
         xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\" \
         xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\">\
         <item id=\"annonce\" parentID=\"-1\" restricted=\"true\">\
         <dc:title>Annonce</dc:title>\
         <upnp:class>object.item.audioItem.musicTrack</upnp:class>\
         <res protocolInfo=\"http-get:*:audio/wav:*\">{}</res></item></DIDL-Lite>",
        xml::escape(url)
    )
}

/// Étapes 1 et 2 : note l'état, puis lance l'annonce. En cas d'échec, les
/// volumes sont remis avant de rendre l'erreur.
pub async fn start(
    api: &Api,
    target: &Target,
    url: &str,
    volume: Option<u8>,
) -> Result<Snapshot, SonosError> {
    let host = target.coordinator.as_str();
    let transport = api.transport_state(host).await?;
    let media = api.media(host).await?;
    let position = api.position(host).await?;
    let mut volumes = Vec::new();
    for member in &target.members {
        volumes.push((
            member.clone(),
            api.volume(member).await?,
            api.muted(member).await?,
        ));
    }
    let snapshot = Snapshot {
        transport,
        media,
        position,
        volumes,
    };
    debug!(
        "Sonos : avant l'annonce sur {host} : {} ({})",
        snapshot.transport, snapshot.media.uri
    );
    let launched = async {
        for (member, current, muted) in &snapshot.volumes {
            if let Some(volume) = volume.filter(|v| v != current) {
                api.set_volume(member, volume).await?;
            }
            // Une annonce en sourdine ne servirait à rien.
            if *muted {
                api.set_mute(member, false).await?;
            }
        }
        api.set_uri(host, url, &metadata(url)).await?;
        api.play(host).await
    }
    .await;
    if let Err(error) = launched {
        restore_volumes(api, &snapshot).await;
        return Err(error);
    }
    Ok(snapshot)
}

/// Étapes 3 et 4 : attend la fin du son (`length`), puis remet tout comme
/// avant.
pub async fn finish(
    api: &Api,
    target: &Target,
    url: &str,
    length: Duration,
    snapshot: &Snapshot,
    timing: Timing,
) {
    let host = target.coordinator.as_str();
    let taken_over = wait_for_the_end(api, host, url, length, timing).await;
    if taken_over {
        info!(
            "Sonos : autre chose a été lancé pendant l'annonce, la musique d'avant n'est pas remise"
        );
    } else {
        restore_media(api, host, snapshot).await;
    }
    restore_volumes(api, snapshot).await;
    let resume = !taken_over
        && snapshot.was_playing()
        && !snapshot.media.uri.is_empty()
        && can_restore(&snapshot.media.uri);
    if resume && let Err(error) = api.play(host).await {
        warn!("Sonos : la musique n'a pas pu reprendre ({error})");
    }
}

/// Attend que l'annonce ait été jouée. Rend `true` si autre chose a pris sa
/// place.
async fn wait_for_the_end(
    api: &Api,
    host: &str,
    url: &str,
    length: Duration,
    timing: Timing,
) -> bool {
    let begin = tokio::time::Instant::now();
    let mut started = false;
    loop {
        tokio::time::sleep(timing.poll).await;
        let elapsed = begin.elapsed();
        let (state, position) = match (api.transport_state(host).await, api.position(host).await) {
            (Ok(state), Ok(position)) => (state, position),
            (Err(error), _) | (_, Err(error)) => {
                debug!("Sonos : état illisible pendant l'annonce ({error})");
                if elapsed > timing.start + length + timing.margin {
                    return false;
                }
                continue;
            }
        };
        let ours = position.track_uri == url;
        let playing = matches!(state.as_str(), "PLAYING" | "TRANSITIONING");
        // Avant d'avoir vu l'annonce jouer, une autre adresse peut être un
        // reste de la musique d'avant : on attend.
        if started && !ours && !position.track_uri.is_empty() {
            return true;
        }
        if ours && playing {
            started = true;
        }
        if started && !playing {
            return false;
        }
        if !started && elapsed > timing.start {
            warn!("Sonos : l'annonce n'a pas démarré (l'enceinte n'a pas pu télécharger le son ?)");
            return false;
        }
        if elapsed > timing.start + length + timing.margin {
            return false;
        }
    }
}

/// Spotify Connect et AirPlay (`x-sonos-vli:`) sont pilotés par le
/// téléphone : on ne peut pas les recharger.
fn can_restore(uri: &str) -> bool {
    !uri.starts_with("x-sonos-vli:")
}

async fn restore_media(api: &Api, host: &str, snapshot: &Snapshot) {
    let media = &snapshot.media;
    if media.uri.is_empty() {
        // Rien n'était chargé : on laisse l'enceinte arrêtée.
        return;
    }
    if !can_restore(&media.uri) {
        info!(
            "Sonos : la musique venait d'un téléphone (Spotify Connect, AirPlay) : à relancer depuis le téléphone"
        );
        return;
    }
    if let Err(error) = api.set_uri(host, &media.uri, &media.metadata).await {
        warn!("Sonos : la musique d'avant n'a pas pu être rechargée ({error})");
        return;
    }
    if media.uri.starts_with("x-rincon-queue:") {
        let position = &snapshot.position;
        if position.track > 0
            && let Err(error) = api
                .seek(host, "TRACK_NR", &position.track.to_string())
                .await
        {
            debug!("Sonos : retour au morceau impossible ({error})");
        }
        if is_time(&position.rel_time)
            && let Err(error) = api.seek(host, "REL_TIME", &position.rel_time).await
        {
            debug!("Sonos : retour au même endroit impossible ({error})");
        }
    }
}

async fn restore_volumes(api: &Api, snapshot: &Snapshot) {
    for (member, volume, muted) in &snapshot.volumes {
        if let Err(error) = api.set_volume(member, *volume).await {
            warn!("Sonos : volume non remis sur {member} ({error})");
        }
        if *muted && let Err(error) = api.set_mute(member, true).await {
            warn!("Sonos : sourdine non remise sur {member} ({error})");
        }
    }
}

/// `0:01:23`, mais pas `0:00:00` ni `NOT_IMPLEMENTED`.
fn is_time(text: &str) -> bool {
    let parts: Vec<&str> = text.split(':').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && parts.iter().any(|p| p.bytes().any(|b| b != b'0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_worth_restoring() {
        assert!(is_time("0:01:23"));
        assert!(is_time("1:00:00"));
        assert!(!is_time("0:00:00"));
        assert!(!is_time("NOT_IMPLEMENTED"));
        assert!(!is_time(""));
        assert!(!can_restore("x-sonos-vli:RINCON_A1:2,spotify:abc"));
        assert!(can_restore("x-rincon-queue:RINCON_A1#0"));
    }

    #[test]
    fn the_metadata_escapes_the_address() {
        let meta = metadata("http://192.168.1.20:8124/annonce/ab.wav?x=1&y=2");
        assert!(meta.contains("x=1&amp;y=2"));
        assert_eq!(
            xml::element_text(&meta, "title").as_deref(),
            Some("Annonce")
        );
    }
}
