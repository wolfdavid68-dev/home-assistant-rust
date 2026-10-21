//! Le petit serveur où les enceintes viennent chercher le son d'une
//! annonce.
//!
//! Une enceinte Sonos ne reçoit pas le son : on lui donne une **adresse**,
//! et elle le télécharge. Le tableau de bord n'écoute souvent que sur le PC
//! lui-même (`127.0.0.1`) : ce serveur-ci écoute sur le réseau local, sur
//! un port à part (8124), et ne sait faire qu'une chose.
//!
//! Sécurité :
//! - il ne sert que les annonces en cours, chacune sous un nom aléatoire
//!   de 128 bits, oublié après [`CLIP_TTL`] ;
//! - seulement aux adresses des enceintes Sonos connues : pour les autres,
//!   rien n'existe (404) ;
//! - aucune autre page, rien du tableau de bord ni de la maison.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use ha_core::CancellationToken;
use tracing::{debug, warn};

/// Port d'écoute par défaut.
pub const DEFAULT_PORT: u16 = 8124;
/// Une annonce est oubliée après ce délai, même si elle n'a pas été lue.
pub const CLIP_TTL: Duration = Duration::from_mins(10);
/// Annonces gardées au plus en même temps.
const MAX_CLIPS: usize = 16;

#[derive(Debug, Default)]
struct Inner {
    clips: HashMap<String, (Bytes, Instant)>,
    allowed: HashSet<IpAddr>,
}

/// Les annonces en cours, et les enceintes qui peuvent les lire.
#[derive(Debug, Default, Clone)]
pub struct Clips(Arc<Mutex<Inner>>);

impl Clips {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Range un son ; rend son nom (`<32 chiffres hexadécimaux>.wav`).
    #[must_use]
    pub fn add(&self, wav: Vec<u8>) -> Option<String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).ok()?;
        let name: String = bytes
            .iter()
            .flat_map(|b| [b >> 4, b & 0x0f])
            .map(|nibble| char::from_digit(u32::from(nibble), 16).unwrap_or('0'))
            .collect();
        let mut inner = self.lock();
        inner
            .clips
            .retain(|_, (_, added)| added.elapsed() < CLIP_TTL);
        while inner.clips.len() >= MAX_CLIPS {
            let oldest = inner
                .clips
                .iter()
                .min_by_key(|(_, (_, added))| *added)
                .map(|(name, _)| name.clone())?;
            inner.clips.remove(&oldest);
        }
        inner
            .clips
            .insert(name.clone(), (Bytes::from(wav), Instant::now()));
        Some(format!("{name}.wav"))
    }

    /// Oublie une annonce finie.
    pub fn remove(&self, file: &str) {
        if let Some(name) = file.strip_suffix(".wav") {
            self.lock().clips.remove(name);
        }
    }

    /// Les adresses des enceintes connues.
    pub fn allow(&self, addresses: HashSet<IpAddr>) {
        self.lock().allowed = addresses;
    }

    fn get(&self, file: &str, from: IpAddr) -> Result<Bytes, Refusal> {
        let inner = self.lock();
        if !inner.allowed.contains(&canonical(from)) {
            return Err(Refusal::Stranger);
        }
        let name = file.strip_suffix(".wav").ok_or(Refusal::Unknown)?;
        match inner.clips.get(name) {
            Some((bytes, added)) if added.elapsed() < CLIP_TTL => Ok(bytes.clone()),
            _ => Err(Refusal::Unknown),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().clips.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

enum Refusal {
    Stranger,
    Unknown,
}

/// Une adresse IPv4 vue en IPv6 (`::ffff:192.168.1.30`) redevient IPv4.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// Sert les annonces jusqu'à l'arrêt.
pub async fn serve(listener: tokio::net::TcpListener, clips: Clips, token: CancellationToken) {
    let app = Router::new()
        .route("/annonce/{file}", get(clip))
        .with_state(clips);
    let served = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move { token.cancelled().await })
    .await;
    if let Err(error) = served {
        warn!("Sonos : le serveur des annonces s'est arrêté ({error})");
    }
}

async fn clip(
    State(clips): State<Clips>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Path(file): Path<String>,
) -> Response {
    match clips.get(&file, from.ip()) {
        Ok(bytes) => {
            debug!("Sonos : annonce téléchargée par {}", from.ip());
            (
                [
                    (header::CONTENT_TYPE, "audio/wav"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                bytes,
            )
                .into_response()
        }
        Err(Refusal::Stranger) => {
            warn!(
                "Sonos : annonce demandée par {}, qui n'est pas une enceinte connue : refusé",
                from.ip()
            );
            StatusCode::NOT_FOUND.into_response()
        }
        Err(Refusal::Unknown) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    #[test]
    fn only_known_speakers_get_current_clips() {
        let clips = Clips::default();
        let file = clips.add(vec![1, 2, 3]).unwrap();
        assert_eq!(file.len(), 32 + 4);
        let speaker: IpAddr = "192.168.1.30".parse().unwrap();
        let stranger: IpAddr = "192.168.1.66".parse().unwrap();
        assert!(matches!(clips.get(&file, speaker), Err(Refusal::Stranger)));
        clips.allow(HashSet::from([speaker]));
        assert_eq!(
            clips.get(&file, speaker).ok().as_deref(),
            Some(&[1u8, 2, 3][..])
        );
        let mapped: IpAddr = "::ffff:192.168.1.30".parse().unwrap();
        assert!(clips.get(&file, mapped).is_ok());
        assert!(matches!(clips.get(&file, stranger), Err(Refusal::Stranger)));
        assert!(matches!(
            clips.get("autre.wav", speaker),
            Err(Refusal::Unknown)
        ));
        assert!(matches!(
            clips.get(file.trim_end_matches(".wav"), speaker),
            Err(Refusal::Unknown)
        ));
        clips.remove(&file);
        assert!(matches!(clips.get(&file, speaker), Err(Refusal::Unknown)));
    }

    #[test]
    fn the_oldest_clip_is_forgotten() {
        let clips = Clips::default();
        let first = clips.add(vec![0]).unwrap();
        for _ in 0..MAX_CLIPS {
            clips.add(vec![1]).unwrap();
        }
        assert_eq!(clips.len(), MAX_CLIPS);
        clips.allow(HashSet::from(["10.0.0.2".parse().unwrap()]));
        assert!(clips.get(&first, "10.0.0.2".parse().unwrap()).is_err());
    }
}
