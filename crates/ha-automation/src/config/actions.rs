//! La vérification des actions (`actions:`), utilisée aussi par les
//! scripts.

use super::MAX_ITEMS;
use super::check::Budget;
use super::conditions::check_condition;
use super::data::DataTemplates;
use super::model::{Action, Condition, MAX_REPEAT, RepeatUntil, Trigger};
use super::parsing::{entity_ids, parse_duration};
use super::read::{ActionConfig, ConditionConfig, RepeatActionConfig};
use super::triggers::check_trigger;

/// Vérifie une action (utilisée aussi par les scripts).
pub(crate) fn check_action(config: ActionConfig) -> Result<Action, String> {
    check_action_in(config, 0, &mut Budget::default())
}

pub(super) fn check_action_in(
    config: ActionConfig,
    depth: usize,
    budget: &mut Budget,
) -> Result<Action, String> {
    budget.take(depth)?;
    let actions = |list: Vec<ActionConfig>, budget: &mut Budget| -> Result<Vec<Action>, String> {
        list.into_iter()
            .map(|a| check_action_in(a, depth + 1, budget))
            .collect()
    };
    let conditions =
        |list: Vec<ConditionConfig>, budget: &mut Budget| -> Result<Vec<Condition>, String> {
            if list.is_empty() {
                return Err("il faut au moins une condition".into());
            }
            list.into_iter()
                .map(|c| check_condition(c, depth + 1, budget))
                .collect()
        };
    match config {
        ActionConfig::Condition(condition) => Ok(Action::Condition(check_condition(
            condition,
            depth + 1,
            budget,
        )?)),
        ActionConfig::If(config) => {
            if config.then.is_empty() {
                return Err("if : il faut au moins une action dans then".into());
            }
            Ok(Action::If {
                conditions: conditions(config.if_, budget)?,
                then: actions(config.then, budget)?,
                otherwise: actions(config.else_.unwrap_or_default(), budget)?,
            })
        }
        ActionConfig::Choose(config) => {
            if config.choose.is_empty() {
                return Err("choose : il faut au moins une option".into());
            }
            let mut options = Vec::with_capacity(config.choose.len());
            for option in config.choose {
                if option.sequence.is_empty() {
                    return Err("choose : chaque option doit avoir une sequence".into());
                }
                options.push((
                    conditions(option.conditions, budget)?,
                    actions(option.sequence, budget)?,
                ));
            }
            Ok(Action::Choose {
                options,
                default: actions(config.default.unwrap_or_default(), budget)?,
            })
        }
        ActionConfig::Repeat(RepeatActionConfig { repeat }) => {
            if repeat.sequence.is_empty() {
                return Err("repeat : il faut au moins une action dans sequence".into());
            }
            let until = match (repeat.count, repeat.while_, repeat.until) {
                (Some(count), None, None) if (1..=MAX_REPEAT).contains(&count) => {
                    RepeatUntil::Count(count)
                }
                (Some(_), None, None) => {
                    return Err(format!("repeat : count entre 1 et {MAX_REPEAT}"));
                }
                (None, Some(list), None) => RepeatUntil::While(conditions(list, budget)?),
                (None, None, Some(list)) => RepeatUntil::Until(conditions(list, budget)?),
                _ => return Err("repeat : il faut count, while ou until (un seul)".into()),
            };
            Ok(Action::Repeat {
                until,
                actions: actions(repeat.sequence, budget)?,
            })
        }
        ActionConfig::WaitForTrigger(wait) => {
            if wait.wait_for_trigger.is_empty() || wait.wait_for_trigger.len() > MAX_ITEMS {
                return Err(format!("wait_for_trigger : 1 à {MAX_ITEMS} déclencheurs"));
            }
            let mut triggers = Vec::with_capacity(wait.wait_for_trigger.len());
            for config in wait.wait_for_trigger {
                budget.take(depth + 1)?;
                let (trigger, _) = check_trigger(config)?;
                if matches!(trigger, Trigger::Sun { .. }) {
                    return Err("wait_for_trigger : le soleil n'est pas possible ici \
                         (utilise une condition sun)"
                        .into());
                }
                triggers.push(trigger);
            }
            Ok(Action::WaitForTrigger {
                triggers,
                timeout: wait.timeout.as_deref().map(parse_duration).transpose()?,
                continue_on_timeout: wait.continue_on_timeout.unwrap_or(true),
            })
        }
        ActionConfig::Delay(delay) => Ok(Action::Delay(parse_duration(&delay.delay)?)),
        ActionConfig::Service(service) => {
            let Some((domain, name)) = service.action.split_once('.') else {
                return Err(format!(
                    "action {:?} : format attendu domaine.service, par exemple light.turn_on",
                    service.action
                ));
            };
            for part in [domain, name] {
                if !ha_core::is_valid_name(part) {
                    return Err(format!("action {:?} invalide", service.action));
                }
            }
            let Some(target) = service.target else {
                return Err(format!(
                    "action {} : il faut une cible (target: entity_id: …)",
                    service.action
                ));
            };
            let templates = DataTemplates::find(&service.data)?;
            Ok(Action::Service {
                domain: domain.to_owned(),
                service: name.to_owned(),
                entity_ids: entity_ids(target.entity_id)?,
                data: service.data,
                templates,
            })
        }
    }
}
