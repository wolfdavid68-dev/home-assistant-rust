//! Les numéros de « lampes Hue » donnés aux appareils, gardés dans la base.

use std::collections::BTreeMap;

use ha_core::EntityId;
use ha_storage::Store;

use crate::config::Device;

/// Donne un numéro à chaque appareil : celui qu'il avait déjà, ou un nouveau.
/// Renvoie numéro → appareil, dans l'ordre des numéros.
///
/// Un appareil retiré de `alexa.yaml` garde son numéro dans la base : s'il
/// revient, Alexa le reconnaîtra.
pub async fn assign(
    store: &Store,
    devices: &[Device],
) -> Result<BTreeMap<u32, Device>, sqlx::Error> {
    let mut tx = store.pool().begin().await?;
    let mut numbers = BTreeMap::new();
    for device in devices {
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT number FROM alexa_lights WHERE entity_id = ?1")
                .bind(device.entity_id.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        let number = if let Some((number,)) = existing {
            number
        } else {
            let (number,): (i64,) = sqlx::query_as(
                "INSERT INTO alexa_lights (entity_id, number)
                 VALUES (?1, (SELECT COALESCE(MAX(number), 0) + 1 FROM alexa_lights))
                 RETURNING number",
            )
            .bind(device.entity_id.as_str())
            .fetch_one(&mut *tx)
            .await?;
            number
        };
        let number = u32::try_from(number)
            .map_err(|_| sqlx::Error::Protocol("numéro de lampe hors limites".into()))?;
        numbers.insert(number, device.clone());
    }
    tx.commit().await?;
    Ok(numbers)
}

/// Retrouve le numéro d'un appareil (pour les tests et la ligne de commande).
#[must_use]
pub fn number_of(numbers: &BTreeMap<u32, Device>, entity_id: &EntityId) -> Option<u32> {
    numbers
        .iter()
        .find(|(_, device)| &device.entity_id == entity_id)
        .map(|(number, _)| *number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse;

    #[tokio::test]
    async fn numbers_never_change() {
        let store = Store::open_in_memory().await.unwrap();
        let first = assign(&store, &parse("- light.a\n- switch.b").unwrap())
            .await
            .unwrap();
        let a: EntityId = "light.a".parse().unwrap();
        let b: EntityId = "switch.b".parse().unwrap();
        assert_eq!(number_of(&first, &a), Some(1));
        assert_eq!(number_of(&first, &b), Some(2));

        // Ordre changé, un appareil retiré, un ajouté : les numéros restent.
        let second = assign(&store, &parse("- cover.c\n- switch.b").unwrap())
            .await
            .unwrap();
        assert_eq!(number_of(&second, &b), Some(2));
        assert_eq!(number_of(&second, &"cover.c".parse().unwrap()), Some(3));
        assert_eq!(number_of(&second, &a), None);
        // Il revient : même numéro qu'avant.
        let third = assign(&store, &parse("- light.a").unwrap()).await.unwrap();
        assert_eq!(number_of(&third, &a), Some(1));
    }
}
