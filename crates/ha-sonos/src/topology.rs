//! Les enceintes de la maison et leurs groupes.
//!
//! N'importe quelle enceinte connaît toutes les autres : on lui demande
//! `GetZoneGroupState`, qui rend les groupes. Dans un groupe, une enceinte
//! est le **coordinateur** : c'est elle qui lit la musique et que l'on
//! commande pour lecture, pause, morceau suivant. Le volume, lui, se règle
//! enceinte par enceinte.
//!
//! On ignore les enceintes « invisibles » : la deuxième d'une paire stéréo,
//! le caisson de basse et les enceintes arrière d'une barre de son. Elles
//! suivent la principale.

use std::net::Ipv4Addr;

use crate::api::PORT;
use crate::xml;

/// Une enceinte visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Son identifiant Sonos, `RINCON_…` : il ne change jamais.
    pub uid: String,
    /// Le nom de la pièce donné dans l'appli Sonos.
    pub name: String,
    /// `192.168.1.30:1400`.
    pub host: String,
}

/// Un groupe : son coordinateur, puis ses enceintes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub coordinator: String,
    pub members: Vec<Member>,
}

impl Group {
    /// L'enceinte qui coordonne.
    #[must_use]
    pub fn coordinator(&self) -> Option<&Member> {
        self.members.iter().find(|m| m.uid == self.coordinator)
    }
}

/// Le nombre d'enceintes gardées au plus.
const MAX_MEMBERS: usize = 64;
const MAX_NAME: usize = 64;

/// Lit la réponse de `GetZoneGroupState`.
#[must_use]
pub fn parse(state: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut count = 0;
    for (name, attributes) in xml::start_tags(state) {
        match name.as_str() {
            "ZoneGroup" => groups.push(Group {
                coordinator: xml::attribute(&attributes, "Coordinator")
                    .unwrap_or_default()
                    .to_owned(),
                members: Vec::new(),
            }),
            "ZoneGroupMember" => {
                let invisible = xml::attribute(&attributes, "Invisible") == Some("1");
                let member = (|| {
                    let uid = xml::attribute(&attributes, "UUID").filter(|u| is_uid(u))?;
                    let host = host_of(xml::attribute(&attributes, "Location")?)?;
                    let name: String = xml::attribute(&attributes, "ZoneName")?
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(MAX_NAME)
                        .collect();
                    Some(Member {
                        uid: uid.to_owned(),
                        name: name.trim().to_owned(),
                        host,
                    })
                })();
                if let (false, Some(member), Some(group)) = (invisible, member, groups.last_mut())
                    && count < MAX_MEMBERS
                {
                    group.members.push(member);
                    count += 1;
                }
            }
            // Les appareils disparus sont listés après : on s'arrête.
            "VanishedDevices" => break,
            _ => {}
        }
    }
    groups.retain(|g| g.coordinator().is_some());
    groups
}

/// `RINCON_` suivi de lettres et de chiffres.
#[must_use]
pub fn is_uid(uid: &str) -> bool {
    uid.len() <= 64
        && uid
            .strip_prefix("RINCON_")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// L'adresse d'une enceinte, d'après l'adresse de sa fiche
/// (`http://192.168.1.30:1400/xml/device_description.xml`).
///
/// Sécurité : seulement une adresse IPv4 **du réseau local** (ou de ce PC)
/// et le port des Sonos. Une réponse trafiquée ne peut pas nous faire
/// appeler une machine sur Internet, ni un autre service.
#[must_use]
pub fn host_of(location: &str) -> Option<String> {
    let url = url::Url::parse(location).ok()?;
    if url.scheme() != "http" || url.port() != Some(PORT) {
        return None;
    }
    let url::Host::Ipv4(ip) = url.host()? else {
        return None;
    };
    is_local(ip).then(|| format!("{ip}:{PORT}"))
}

fn is_local(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || ip.is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = r#"<ZoneGroupState><ZoneGroups>
<ZoneGroup Coordinator="RINCON_A1" ID="RINCON_A1:1">
  <ZoneGroupMember UUID="RINCON_A1" Location="http://192.168.1.30:1400/xml/device_description.xml" ZoneName="Salon">
    <Satellite UUID="RINCON_SUB" Location="http://192.168.1.33:1400/xml/device_description.xml" ZoneName="Salon" Invisible="1"/>
  </ZoneGroupMember>
  <ZoneGroupMember UUID="RINCON_A2" Location="http://192.168.1.31:1400/xml/device_description.xml" ZoneName="Salon" Invisible="1"/>
  <ZoneGroupMember UUID="RINCON_B1" Location="http://192.168.1.32:1400/xml/device_description.xml" ZoneName="Cuisine"/>
</ZoneGroup>
<ZoneGroup Coordinator="RINCON_C1" ID="RINCON_C1:2">
  <ZoneGroupMember UUID="RINCON_C1" Location="http://192.168.1.34:1400/xml/device_description.xml" ZoneName="Chambre d&apos;amis"/>
</ZoneGroup>
<ZoneGroup Coordinator="RINCON_X" ID="x">
  <ZoneGroupMember UUID="RINCON_X" Location="http://8.8.8.8:1400/xml/device_description.xml" ZoneName="Ailleurs"/>
</ZoneGroup>
</ZoneGroups><VanishedDevices><ZoneGroupMember UUID="RINCON_V" Location="http://192.168.1.99:1400/x" ZoneName="Disparue"/></VanishedDevices></ZoneGroupState>"#;

    #[test]
    fn groups_and_visible_speakers() {
        let groups = parse(STATE);
        assert_eq!(groups.len(), 2, "le groupe hors du réseau local est ignoré");
        assert_eq!(groups[0].coordinator, "RINCON_A1");
        let names: Vec<&str> = groups[0].members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["Salon", "Cuisine"],
            "ni le caisson ni la paire stéréo"
        );
        assert_eq!(groups[0].members[1].host, "192.168.1.32:1400");
        assert_eq!(
            groups[1].coordinator().map(|m| m.name.as_str()),
            Some("Chambre d'amis")
        );
    }

    #[test]
    fn only_local_sonos_addresses() {
        assert_eq!(
            host_of("http://10.0.0.5:1400/xml/device_description.xml").as_deref(),
            Some("10.0.0.5:1400")
        );
        for bad in [
            "http://8.8.8.8:1400/x",
            "http://192.168.1.30:80/x",
            "https://192.168.1.30:1400/x",
            "http://sonos.local:1400/x",
            "http://user@192.168.1.30:1400/x",
            "pas une adresse",
        ] {
            // Le nom d'utilisateur ne change pas l'hôte : accepté, mais
            // l'adresse rendue ne le contient pas.
            let host = host_of(bad);
            assert!(
                host.is_none() || host.as_deref() == Some("192.168.1.30:1400"),
                "{bad}"
            );
        }
        assert!(is_uid("RINCON_000E58C0FFEE01400"));
        assert!(!is_uid("RINCON_"));
        assert!(!is_uid("RINCON_a/b"));
        assert!(!is_uid("uuid:123"));
    }
}
