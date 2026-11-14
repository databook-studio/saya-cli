//! The reload plan record and the per-target reseed comparisons.

use super::classify::{NEXT_SESSION_NOTE, ReseedTarget};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::SessionState;
use saya_config::AiProvider;
use serde_json::{Value, json};

/// One changed user-facing setting: redacted values plus an optional note.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChangeLine {
    pub key: String,
    pub before: Value,
    pub after: Value,
    pub note: Option<String>,
}

impl ChangeLine {
    pub(crate) fn new(key: &str, before: &Value, after: &Value, note: Option<&str>) -> Self {
        Self {
            key: key.to_owned(),
            before: before.clone(),
            after: after.clone(),
            note: note.map(str::to_owned),
        }
    }
}

/// The session-level copies `apply_plan` may overwrite, only on reseed.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct StateReseed {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub profile: Option<Option<String>>,
    pub provider_endpoint: Option<Option<String>>,
}

impl StateReseed {
    pub(crate) fn write(&mut self, target: ReseedTarget, new: &RuntimeConfig) {
        match target {
            ReseedTarget::Provider => {
                self.provider = Some(new.resolved.ai.provider.as_str().to_owned())
            }
            ReseedTarget::Model => self.model = Some(new.resolved.ai.model.clone()),
            ReseedTarget::Profile => self.profile = Some(new.resolved.profile_name.clone()),
            ReseedTarget::Endpoint => {
                self.provider_endpoint = Some(new.resolved.ai.base_url.clone())
            }
        }
    }
}

impl ReseedTarget {
    pub(crate) fn already_matches(self, new: &RuntimeConfig, state: &SessionState) -> bool {
        match self {
            ReseedTarget::Provider => {
                AiProvider::parse(&state.provider) == Some(new.resolved.ai.provider)
            }
            ReseedTarget::Model => state.model == new.resolved.ai.model,
            ReseedTarget::Profile => state.profile == new.resolved.profile_name,
            ReseedTarget::Endpoint => {
                state.provider_endpoint.as_deref() == new.resolved.ai.base_url.as_deref()
            }
        }
    }

    /// Overridden ⇔ the session copy differs from the old runtime's value.
    pub(crate) fn user_overrode(self, old: &RuntimeConfig, state: &SessionState) -> bool {
        match self {
            ReseedTarget::Provider => {
                AiProvider::parse(&state.provider) != Some(old.resolved.ai.provider)
            }
            ReseedTarget::Model => state.model != old.resolved.ai.model,
            ReseedTarget::Profile => state.profile != old.resolved.profile_name,
            ReseedTarget::Endpoint => {
                state.provider_endpoint_bound
                    && state.provider_endpoint.as_deref() != old.resolved.ai.base_url.as_deref()
            }
        }
    }
}

/// The classification of one reload: applied (next turn), reseeded (session
/// state now), pending (does not change this session).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ReloadPlan {
    pub applied: Vec<ChangeLine>,
    pub reseeded: Vec<ChangeLine>,
    pub pending: Vec<ChangeLine>,
    /// The candidate's ignored project overrides, reported once.
    pub ignored_project_overrides: Vec<String>,
    pub reseed: StateReseed,
}

impl ReloadPlan {
    /// Nothing changed anywhere — the report is the unchanged line.
    pub(crate) fn is_empty(&self) -> bool {
        self.applied.is_empty() && self.reseeded.is_empty() && self.pending.is_empty()
    }

    /// The profile registry is read per turn: one applied `connections` change.
    pub(crate) fn connections_change(&mut self, old: &RuntimeConfig, new: &RuntimeConfig) {
        if old.connections.profiles == new.connections.profiles {
            return;
        }
        self.applied.push(ChangeLine {
            key: "connections".to_owned(),
            before: profile_names(&old.connections),
            after: profile_names(&new.connections),
            note: None,
        });
    }

    /// Host-command shaping and session denies are frozen at composition.
    pub(crate) fn command_shaping_changes(&mut self, old: &RuntimeConfig, new: &RuntimeConfig) {
        if old.resolved.host_commands != new.resolved.host_commands {
            self.pending.push(ChangeLine {
                key: "host_commands".to_owned(),
                before: host_commands_json(&old.resolved.host_commands),
                after: host_commands_json(&new.resolved.host_commands),
                note: Some(NEXT_SESSION_NOTE.to_owned()),
            });
        }
        if old.resolved.session_deny != new.resolved.session_deny {
            self.pending.push(ChangeLine {
                key: "session_commands".to_owned(),
                before: json!({ "deny": old.resolved.session_deny.programs }),
                after: json!({ "deny": new.resolved.session_deny.programs }),
                note: Some(NEXT_SESSION_NOTE.to_owned()),
            });
        }
    }
}

fn profile_names(connections: &saya_config::ConnectionsFile) -> Value {
    Value::Array(
        connections
            .profiles
            .keys()
            .map(|name| Value::String(name.clone()))
            .collect(),
    )
}

fn host_commands_json(commands: &saya_config::ResolvedHostCommands) -> Value {
    json!({
        "pass_env": commands.pass_env,
        "timeout_seconds": commands.timeout_seconds,
    })
}
