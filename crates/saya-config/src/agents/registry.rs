//! The helper agent-type registry: the two built-in `subagent_type` shapes plus
//! whatever trusted `[agents]` adds, under the rule that a declared entry may
//! only narrow a type that already exists.
//!
//! The registry decides a helper's *shape*, never its authority: the harness
//! still narrows every child against the job's own scope, so a type that
//! declares `implementer` writes only where the job already could.

use std::collections::BTreeMap;

use crate::{AgentFile, AgentRole, ConfigError, MAX_ENDPOINT_STRING_CHARS};

use super::limits::{MAX_HELPER_AGENT_NAME_CHARS, MAX_HELPER_AGENTS, RESERVED_AGENT_SETTINGS};

/// The built-in read-only helper type.
pub const EXPLORE_AGENT: &str = "explore";
/// The built-in write-capable helper type.
pub const GENERAL_AGENT: &str = "general";

/// One helper type: the role it composes as, whether it may write, and the
/// model its helpers ask for when the config names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperAgent {
    pub role: AgentRole,
    pub allow_write: bool,
    /// The model this type's helpers send, or `None` — the default — for the
    /// job's own model, which is the request a helper always sent. It selects
    /// which model answers on the job's endpoint; it never names a provider.
    pub model: Option<String>,
}

/// Every helper type one job's `helpers` tool advertises, keyed by the
/// `subagent_type` a call sends and ordered by name, so the advertised schema
/// and a refusal naming the known types cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperAgents(BTreeMap<String, HelperAgent>);

impl Default for HelperAgents {
    fn default() -> Self {
        Self::builtin()
    }
}

impl HelperAgents {
    /// The two built-ins: `explore` reads only, `general` implements one piece.
    /// A job whose config declares no `[agents]` entry runs exactly these.
    pub fn builtin() -> Self {
        Self(BTreeMap::from([
            (
                EXPLORE_AGENT.to_owned(),
                HelperAgent {
                    role: AgentRole::Analyst,
                    allow_write: false,
                    model: None,
                },
            ),
            (
                GENERAL_AGENT.to_owned(),
                HelperAgent {
                    role: AgentRole::Implementer,
                    allow_write: true,
                    model: None,
                },
            ),
        ]))
    }

    pub fn get(&self, name: &str) -> Option<&HelperAgent> {
        self.0.get(name)
    }

    /// Every type, in advertisement order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &HelperAgent)> {
        self.0.iter().map(|(name, agent)| (name.as_str(), agent))
    }

    /// The known type names, in advertisement order: what a refusal names.
    pub fn known_names(&self) -> String {
        self.0.keys().cloned().collect::<Vec<_>>().join(", ")
    }
}

/// Resolves the registry: the built-ins, then every declared entry, each of
/// which may only narrow a type it names.
pub(crate) fn resolve(declared: &BTreeMap<String, AgentFile>) -> Result<HelperAgents, ConfigError> {
    let mut agents = HelperAgents::builtin();
    if declared.is_empty() {
        return Ok(agents);
    }
    if declared.len() > MAX_HELPER_AGENTS {
        return Err(ConfigError::SettingAboveMaximum {
            field: "agents",
            value: declared.len(),
            max: MAX_HELPER_AGENTS,
        });
    }
    for (name, entry) in declared {
        require_name(name)?;
        let base = agents.0.get(name).cloned();
        let role = entry
            .role
            .or(base.as_ref().map(|agent| agent.role))
            .unwrap_or(AgentRole::Analyst);
        let allow_write = entry.allow_write.unwrap_or(role == AgentRole::Implementer);
        if allow_write && role == AgentRole::Analyst {
            return Err(ConfigError::InvalidAgentType {
                name: name.clone(),
                reason:
                    "an analyst helper reads only; state role = \"implementer\" to let it write"
                        .to_owned(),
            });
        }
        if let Some(base) = base.as_ref()
            && (role == AgentRole::Implementer && base.role == AgentRole::Analyst
                || allow_write && !base.allow_write)
        {
            return Err(ConfigError::InvalidAgentType {
                name: name.clone(),
                reason: "a declared entry may narrow a helper type, never widen it".to_owned(),
            });
        }
        if let Some(model) = entry.model.as_deref() {
            require_model(name, model)?;
        }
        agents.0.insert(
            name.clone(),
            HelperAgent {
                role,
                allow_write,
                // The rule the other two fields follow: an entry states what it
                // changes. A model is not a narrowing — it decides which model
                // answers a helper, never what the helper may do — so an entry
                // may state one freely.
                model: entry
                    .model
                    .clone()
                    .or_else(|| base.and_then(|agent| agent.model)),
            },
        );
    }
    Ok(agents)
}

/// A helper's model name is copied into a provider request, so it carries the
/// same bound an endpoint's model carries (`MAX_ENDPOINT_STRING_CHARS`): the
/// same value, in the same place, checked before it can reach one.
fn require_model(name: &str, model: &str) -> Result<(), ConfigError> {
    let value = model.chars().count();
    if value <= MAX_ENDPOINT_STRING_CHARS {
        Ok(())
    } else {
        Err(ConfigError::AgentModelTooLong {
            name: name.to_owned(),
            value,
            max: MAX_ENDPOINT_STRING_CHARS,
        })
    }
}

/// A helper type name is advertised in the tool schema and echoed in a refusal,
/// so it is a bounded lowercase identifier — and never one of the two settings
/// the same table holds.
fn require_name(name: &str) -> Result<(), ConfigError> {
    if RESERVED_AGENT_SETTINGS.contains(&name) {
        return Err(ConfigError::InvalidAgentType {
            name: name.to_owned(),
            reason: "this name is reserved: [agents] max_children and \
                     [agents] max_concurrent_model_requests are settings, not helper types"
                .to_owned(),
        });
    }
    let mut chars = name.chars();
    let shaped = name.chars().count() <= MAX_HELPER_AGENT_NAME_CHARS
        && chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|rest| {
            rest.is_ascii_lowercase() || rest.is_ascii_digit() || matches!(rest, '-' | '_')
        });
    if shaped {
        Ok(())
    } else {
        Err(ConfigError::InvalidAgentType {
            name: name.to_owned(),
            reason: "a helper type name is up to 32 ASCII lowercase letters, digits, `-` or `_`, \
                     starting with a letter"
                .to_owned(),
        })
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
