//! La vérification des conditions (`conditions:`, et celles des blocs
//! `if`, `choose`, `repeat`).

use crate::template::Template;

use super::check::{Budget, check_label};
use super::model::Condition;
use super::parsing::{
    check_range, entity_ids, parse_offset, parse_sun_event, parse_time, parse_weekday, values,
};
use super::read::{ConditionConfig, OneOrMany};

pub(super) fn check_condition(
    config: ConditionConfig,
    depth: usize,
    budget: &mut Budget,
) -> Result<Condition, String> {
    budget.take(depth)?;
    let mut nested = |list: Vec<ConditionConfig>, name: &str| -> Result<Vec<Condition>, String> {
        if list.is_empty() {
            return Err(format!("condition {name} : il faut au moins une condition"));
        }
        list.into_iter()
            .map(|c| check_condition(c, depth + 1, budget))
            .collect()
    };
    Ok(match config {
        ConditionConfig::And { conditions } => Condition::And(nested(conditions, "and")?),
        ConditionConfig::Or { conditions } => Condition::Or(nested(conditions, "or")?),
        ConditionConfig::Not { conditions } => Condition::Not(nested(conditions, "not")?),
        ConditionConfig::Template { value_template } => {
            let template = Template::parse(&value_template)
                .map_err(|error| format!("condition template : {error}"))?
                .ok_or("condition template : un modèle {{ … }} attendu dans value_template")?;
            Condition::Template {
                source: value_template,
                template,
            }
        }
        ConditionConfig::Trigger { id } => Condition::Trigger(
            id.into_vec()
                .iter()
                .map(|id| check_label(id, "id de déclencheur"))
                .collect::<Result<_, _>>()?,
        ),
        ConditionConfig::State {
            entity_id,
            attribute,
            state,
        } => Condition::State {
            entity_ids: entity_ids(entity_id)?,
            attribute: attribute
                .map(|name| check_label(&name, "attribute"))
                .transpose()?,
            states: values(Some(state), "state")?,
        },
        ConditionConfig::NumericState {
            entity_id,
            above,
            below,
        } => {
            check_range(above, below)?;
            Condition::NumericState {
                entity_ids: entity_ids(entity_id)?,
                above,
                below,
            }
        }
        ConditionConfig::Time {
            after,
            before,
            weekday,
        } => {
            if after.is_none() && before.is_none() && weekday.is_none() {
                return Err("condition time : il faut after, before ou weekday".into());
            }
            Condition::Time {
                after: after.as_deref().map(parse_time).transpose()?,
                before: before.as_deref().map(parse_time).transpose()?,
                weekdays: weekday
                    .map(OneOrMany::into_vec)
                    .unwrap_or_default()
                    .iter()
                    .map(|day| parse_weekday(day))
                    .collect::<Result<_, _>>()?,
            }
        }
        ConditionConfig::Sun {
            after,
            before,
            after_offset,
            before_offset,
        } => {
            if after.is_none() && before.is_none() {
                return Err("condition sun : il faut after ou before".into());
            }
            let with_offset = |event: Option<String>, offset: Option<String>| {
                if event.is_none() && offset.is_some() {
                    return Err("condition sun : décalage sans événement".to_owned());
                }
                event
                    .map(|event| {
                        Ok((
                            parse_sun_event(&event)?,
                            offset
                                .as_deref()
                                .map(parse_offset)
                                .transpose()?
                                .unwrap_or_default(),
                        ))
                    })
                    .transpose()
            };
            Condition::Sun {
                after: with_offset(after, after_offset)?,
                before: with_offset(before, before_offset)?,
            }
        }
    })
}
