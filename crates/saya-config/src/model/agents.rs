use std::collections::BTreeMap;

use serde::Deserialize;

/// Raw trusted `[agents]` settings: the two bounds a job's helper scheduler
/// runs under, and the helper type registry, in one table.
///
/// The table holds both because they are one subject — the helper lane a coding
/// job may open — and because both are the operator's word about it: a key that
/// is not one of the two settings is a helper type, and the two setting names
/// are reserved so a key never means two things at once. The whole table is on
/// the protected list, so an untrusted project layer can neither add or
/// retarget a type nor raise either bound.
///
/// `#[serde(flatten)]` is what lets one table hold both: serde cannot hold a
/// named map beside named scalars under `deny_unknown_fields`, so the entries
/// are the leftovers. The shape mistakes that fall out of that — a setting
/// written as a table, a type written as a scalar — are named in
/// `ConfigFile::from_toml`, which reads the text the way a retired key is read.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct AgentsFile {
    pub max_children: Option<usize>,
    pub max_concurrent_model_requests: Option<usize>,
    /// The helper types a `helpers` call may name, keyed by the `subagent_type`
    /// it sends. Absent — the default — leaves the job with the two built-ins,
    /// `explore` and `general`.
    #[serde(default, flatten)]
    pub types: BTreeMap<String, AgentFile>,
}

/// One `[agents.<name>]` entry. Every field is optional: an entry states only
/// what it changes about the type it names.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentFile {
    /// The role the helper composes as. Absent keeps the role the type already
    /// has; a type the config introduces starts as `analyst`.
    pub role: Option<AgentRole>,
    /// Whether the helper may write the job's working checkout. `false` narrows
    /// a write-capable type to read-only; `true` cannot widen a read-only one.
    /// Absent follows the resolved role.
    pub allow_write: Option<bool>,
    /// The model this type's helpers ask for. Absent — the default — leaves
    /// them on the job's own model, so the request is the one it always was.
    /// It selects what answers; the endpoint, base URL and credential stay the
    /// job's, so it can never point a helper at another provider. Bounded like
    /// an endpoint's model at resolution.
    pub model: Option<String>,
}

/// The two helper shapes an `[agents]` entry can name. Neither grants
/// authority: the harness narrows every child against the job's own scope, so a
/// `role` can only decide which of that scope's capabilities a helper uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Reads and searches; the harness narrows it to a read-only view.
    Analyst,
    /// Implements one piece, within the job's own write ceiling.
    Implementer,
}

impl AgentRole {
    /// The name this role is spelled with in `[agents]`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Analyst => "analyst",
            Self::Implementer => "implementer",
        }
    }
}
