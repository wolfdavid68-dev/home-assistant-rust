//! Les modèles `{{ … }}` dans les données (`data:`) d'une action : où ils
//! sont, et les données une fois calculées.

use ha_core::{Attributes, EntityId};
use serde_json::Value;

use crate::template::{Scope, Template};

/// Où se trouve un modèle dans `data` : `message`, `data › 0 › titre`…
#[derive(Debug, Clone, PartialEq)]
enum DataKey {
    Field(String),
    Index(usize),
}

/// Les modèles d'une action (`{{ … }}` dans `data`), et leur place.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataTemplates(Vec<(Vec<DataKey>, Template)>);

/// Profondeur maximale explorée dans `data`, et nombre de modèles au plus.
const MAX_DATA_DEPTH: usize = 4;
const MAX_TEMPLATES: usize = 10;

impl DataTemplates {
    /// Trouve et vérifie les modèles dans `data`.
    pub(super) fn find(data: &Attributes) -> Result<Self, String> {
        let mut found = Vec::new();
        for (key, value) in data {
            find_templates(value, &mut vec![DataKey::Field(key.clone())], &mut found)?;
        }
        if found.len() > MAX_TEMPLATES {
            return Err(format!("data : {MAX_TEMPLATES} modèles au plus"));
        }
        Ok(Self(found))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Les données avec les modèles calculés.
    pub(crate) fn render(&self, data: &Attributes, scope: &Scope<'_>) -> Attributes {
        let mut data = data.clone();
        for (path, template) in &self.0 {
            let Some((DataKey::Field(first), rest)) = path.split_first() else {
                continue;
            };
            if let Some(slot) = data.get_mut(first).and_then(|value| slot_at(value, rest)) {
                *slot = template.render(scope);
            }
        }
        data
    }

    pub(super) fn entity_ids(&self) -> impl Iterator<Item = &EntityId> {
        self.0
            .iter()
            .flat_map(|(_, template)| template.entity_ids())
    }
}

/// La valeur à la place `path` dans `value`.
fn slot_at<'v>(value: &'v mut Value, path: &[DataKey]) -> Option<&'v mut Value> {
    let Some((key, rest)) = path.split_first() else {
        return Some(value);
    };
    let next = match (key, value) {
        (DataKey::Field(name), Value::Object(map)) => map.get_mut(name),
        (DataKey::Index(index), Value::Array(list)) => list.get_mut(*index),
        _ => None,
    };
    slot_at(next?, rest)
}

fn find_templates(
    value: &Value,
    path: &mut Vec<DataKey>,
    found: &mut Vec<(Vec<DataKey>, Template)>,
) -> Result<(), String> {
    if path.len() > MAX_DATA_DEPTH {
        return Ok(());
    }
    match value {
        Value::String(text) => {
            if let Some(template) = Template::parse(text).map_err(|error| {
                let place: Vec<String> = path
                    .iter()
                    .map(|key| match key {
                        DataKey::Field(name) => name.clone(),
                        DataKey::Index(index) => index.to_string(),
                    })
                    .collect();
                format!("data.{} : {error}", place.join("."))
            })? {
                found.push((path.clone(), template));
            }
        }
        Value::Array(list) => {
            for (index, item) in list.iter().enumerate() {
                path.push(DataKey::Index(index));
                find_templates(item, path, found)?;
                path.pop();
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                path.push(DataKey::Field(key.clone()));
                find_templates(item, path, found)?;
                path.pop();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}
