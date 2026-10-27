//! Ce qu'une automatisation cite : ses entités, et le soleil (qui demande
//! la position de la maison).

use ha_core::EntityId;

use super::model::{Action, Automation, Condition, RepeatUntil, Trigger};

/// Les automatisations utilisent-elles le soleil ? (Il faut alors la
/// position de la maison.)
#[must_use]
pub fn uses_sun(automations: &[Automation]) -> bool {
    automations.iter().any(|automation| {
        automation
            .triggers
            .iter()
            .any(|t| matches!(t, Trigger::Sun { .. }))
            || automation.conditions.iter().any(Condition::uses_sun)
            || automation.actions.iter().any(Action::uses_sun)
    })
}

impl Condition {
    fn uses_sun(&self) -> bool {
        match self {
            Self::Sun { .. } => true,
            Self::And(list) | Self::Or(list) | Self::Not(list) => list.iter().any(Self::uses_sun),
            _ => false,
        }
    }

    fn collect_entities<'a>(&'a self, ids: &mut Vec<&'a EntityId>) {
        match self {
            Self::State { entity_ids, .. } | Self::NumericState { entity_ids, .. } => {
                ids.extend(entity_ids);
            }
            Self::And(list) | Self::Or(list) | Self::Not(list) => {
                for condition in list {
                    condition.collect_entities(ids);
                }
            }
            Self::Template { template, .. } => ids.extend(template.entity_ids()),
            _ => {}
        }
    }
}

impl Action {
    fn uses_sun(&self) -> bool {
        match self {
            Self::Condition(condition) => condition.uses_sun(),
            Self::If {
                conditions,
                then,
                otherwise,
            } => {
                conditions.iter().any(Condition::uses_sun)
                    || then.iter().chain(otherwise).any(Self::uses_sun)
            }
            Self::Choose { options, default } => {
                options.iter().any(|(conditions, actions)| {
                    conditions.iter().any(Condition::uses_sun) || actions.iter().any(Self::uses_sun)
                }) || default.iter().any(Self::uses_sun)
            }
            Self::Repeat { until, actions } => {
                let conditions = match until {
                    RepeatUntil::Count(_) => &[][..],
                    RepeatUntil::While(list) | RepeatUntil::Until(list) => list,
                };
                conditions.iter().any(Condition::uses_sun) || actions.iter().any(Self::uses_sun)
            }
            Self::Service { .. } | Self::Delay(_) | Self::WaitForTrigger { .. } => false,
        }
    }

    fn collect_entities<'a>(&'a self, ids: &mut Vec<&'a EntityId>) {
        match self {
            Self::Service {
                entity_ids,
                templates,
                ..
            } => {
                ids.extend(entity_ids);
                ids.extend(templates.entity_ids());
            }
            Self::Delay(_) => {}
            Self::WaitForTrigger { triggers, .. } => {
                for trigger in triggers {
                    if let Trigger::State { entity_ids, .. }
                    | Trigger::NumericState { entity_ids, .. } = trigger
                    {
                        ids.extend(entity_ids);
                    }
                }
            }
            Self::Repeat { until, actions } => {
                if let RepeatUntil::While(list) | RepeatUntil::Until(list) = until {
                    for condition in list {
                        condition.collect_entities(ids);
                    }
                }
                for action in actions {
                    action.collect_entities(ids);
                }
            }
            Self::Condition(condition) => condition.collect_entities(ids),
            Self::If {
                conditions,
                then,
                otherwise,
            } => {
                for condition in conditions {
                    condition.collect_entities(ids);
                }
                for action in then.iter().chain(otherwise) {
                    action.collect_entities(ids);
                }
            }
            Self::Choose { options, default } => {
                for (conditions, actions) in options {
                    for condition in conditions {
                        condition.collect_entities(ids);
                    }
                    for action in actions {
                        action.collect_entities(ids);
                    }
                }
                for action in default {
                    action.collect_entities(ids);
                }
            }
        }
    }
}

impl Automation {
    /// Toutes les entités citées (déclencheurs, conditions, actions), triées.
    #[must_use]
    pub fn entity_ids(&self) -> Vec<&EntityId> {
        let mut ids: Vec<&EntityId> = Vec::new();
        for trigger in &self.triggers {
            if let Trigger::State { entity_ids, .. } | Trigger::NumericState { entity_ids, .. } =
                trigger
            {
                ids.extend(entity_ids);
            }
        }
        for condition in &self.conditions {
            condition.collect_entities(&mut ids);
        }
        for action in &self.actions {
            action.collect_entities(&mut ids);
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}
