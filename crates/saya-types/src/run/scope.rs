//! The capability scopes a run declares and a plan's steps must stay inside.
//!
//! The run's `Capabilities` are the approval surface for the whole run: what
//! the user (or a headless `--allow`) approved once, at `planned → approved`.
//! A plan's steps are validated against it — a step asking for a scope the
//! run was not approved for is rejected, never silently granted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::RunContractError;
use crate::MAX_NAME_CHARS;

pub const MAX_ENDPOINT_BINDINGS: usize = 8;
pub const MAX_FETCH_DESTINATIONS: usize = 32;
pub const MAX_RUNNER_PROGRAMS: usize = 32;

const MAX_HOST_CHARS: usize = 253;

/// True when `name` has the shape a run-scoped name must have: non-empty,
/// bounded, and free of control characters and whitespace. Roles, endpoint
/// names, budget keys, and runner programs all share this shape. Exported so
/// config resolution can reject a malformed `[jobs]` budget key at resolve
/// time with this exact rule, instead of duplicating it.
pub fn is_name_shaped(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// True when `name` is a bare program name the runner can resolve against
/// one program directory: the run-scoped name shape with no path separators
/// and no relative forms. A runner allowlist names programs, never paths —
/// an absolute path or a traversal is a refusal at every layer (config
/// resolution, `RunnerScope`, the tool), because a path-shaped "name" could
/// point anywhere on the filesystem while the sandbox only allows exec
/// inside the program directory. The same rule an artifact name must
/// satisfy, shared so the two cannot drift.
pub fn is_bare_name(name: &str) -> bool {
    is_name_shaped(name)
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
}

/// One declared fetch destination: a scheme plus a bare host. The fetch
/// policy (a later milestone) decides which schemes and hosts are actually
/// reachable; this contract only demands a shape that cannot smuggle a path,
/// credentials, or control characters into the approval view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Destination {
    /// URL scheme, e.g. `https` — never a full URL.
    pub scheme: String,
    /// Hostname only: no scheme, path, userinfo, or port.
    pub host: String,
}

impl Destination {
    pub fn new(
        scheme: impl Into<String>,
        host: impl Into<String>,
    ) -> Result<Self, RunContractError> {
        let scheme = scheme.into();
        let host = host.into();
        let scheme_shaped = !scheme.is_empty()
            && scheme.len() <= 32
            && scheme.bytes().all(|b| b.is_ascii_alphanumeric());
        let host_shaped = !host.is_empty()
            && host.chars().count() <= MAX_HOST_CHARS
            && !host
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '/' || c == '\\');
        if !scheme_shaped || !host_shaped {
            return Err(RunContractError::InvalidDestination);
        }
        Ok(Self { scheme, host })
    }
}

/// The fetch capability with its declared destinations. A fetch scope with no
/// destinations is a declared capability that can reach nothing, so it is
/// rejected — egress is approved per destination or not at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FetchScope {
    pub destinations: Vec<Destination>,
}

impl FetchScope {
    pub fn new(destinations: Vec<Destination>) -> Result<Self, RunContractError> {
        if destinations.is_empty() {
            return Err(RunContractError::EmptyDestinations);
        }
        if destinations.len() > MAX_FETCH_DESTINATIONS {
            return Err(RunContractError::TooManyDestinations);
        }
        Ok(Self { destinations })
    }
}

/// The programs a runner step may never name, whatever any allowlist says:
/// shells and interpreters. The runner's contract is typed argv against one
/// allowlisted program — an interpreter can spawn arbitrary children with
/// arbitrary argv, so allowing one would void that contract from inside the
/// allowlist. Shared by config resolution (which refuses the name at resolve
/// time) and the runner tool (which refuses the call even if a hand-built
/// scope carries it), so the two layers cannot disagree.
pub fn is_refused_runner_program(name: &str) -> bool {
    const REFUSED: &[&str] = &[
        "sh",
        "bash",
        "dash",
        "zsh",
        "ksh",
        "csh",
        "tcsh",
        "fish",
        "env",
        "perl",
        "python",
        "python3",
        "ruby",
        "node",
        "php",
        "lua",
        "awk",
        "osascript",
        "expect",
        "tclsh",
        "swift",
        "script",
    ];
    REFUSED.contains(&name)
}

/// The runner capability with its allowlisted programs. Typed argv and shell
/// refusal are the runner's own concern; this scope is the universe of
/// programs a run's plan may narrow a step to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunnerScope {
    pub programs: Vec<String>,
}

impl RunnerScope {
    pub fn new(programs: Vec<String>) -> Result<Self, RunContractError> {
        if programs.is_empty() {
            return Err(RunContractError::EmptyPrograms);
        }
        if programs.len() > MAX_RUNNER_PROGRAMS {
            return Err(RunContractError::TooManyPrograms);
        }
        // A scope names programs, never paths: every entry must be a bare
        // name the runner can resolve against one program directory. A
        // path-shaped entry — an absolute path, a traversal, a separator —
        // could point anywhere on the filesystem while the sandbox only
        // allows exec inside the program directory, so it is refused here
        // rather than relied on the tool to catch at call time.
        if !programs.iter().all(|p| is_bare_name(p)) {
            return Err(RunContractError::InvalidProgram);
        }
        Ok(Self { programs })
    }
}

/// Roles bound to named endpoints, e.g. `orchestrator → primary`. Both sides
/// of a binding are run-scoped names; entries are bounded so a plan cannot
/// smuggle an unbounded approval view.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<String, String>")]
pub struct EndpointBindings(BTreeMap<String, String>);

impl EndpointBindings {
    pub fn new<K: Into<String>, V: Into<String>>(
        bindings: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self, RunContractError> {
        let mut map = BTreeMap::new();
        for (role, endpoint) in bindings {
            let (role, endpoint) = (role.into(), endpoint.into());
            if !is_name_shaped(&role) || !is_name_shaped(&endpoint) {
                return Err(RunContractError::InvalidEndpointName);
            }
            map.insert(role, endpoint);
        }
        if map.len() > MAX_ENDPOINT_BINDINGS {
            return Err(RunContractError::TooManyEndpointBindings);
        }
        Ok(Self(map))
    }

    pub fn get(&self, role: &str) -> Option<&str> {
        self.0.get(role).map(String::as_str)
    }

    pub fn contains(&self, role: &str) -> bool {
        self.0.contains_key(role)
    }

    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.0
    }

    pub fn is_subset_of(&self, approved: &Self) -> bool {
        self.0
            .iter()
            .all(|(role, endpoint)| approved.0.get(role) == Some(endpoint))
    }
}

impl TryFrom<BTreeMap<String, String>> for EndpointBindings {
    type Error = RunContractError;

    fn try_from(map: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        Self::new(map)
    }
}

/// The scopes a run is approved for. `Default` is "no capability at all";
/// the composition root builds an approval from CLI flags or TUI choices by
/// starting there and turning on what was approved. Fields are public so an
/// approval can be assembled, but every step is checked against the approved
/// set by [`RunPlan::validate`](super::RunPlan::validate).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Capabilities {
    /// The run may write files inside its own run workspace — nowhere else.
    pub workspace_write: bool,
    /// The declared fetch destinations; `None` means no fetch capability.
    pub fetch: Option<FetchScope>,
    /// The allowlisted programs; `None` means no runner capability.
    pub runner: Option<RunnerScope>,
    /// The run-scoped scratch database.
    pub scratch: bool,
    /// The roles bound to endpoints this run may call.
    pub endpoints: EndpointBindings,
}

impl Capabilities {
    /// True when every scope `self` asks for is approved by `approved`. A
    /// scope that names a set (destinations, programs, bindings) must be a
    /// subset of the approved set — naming a member the run was not approved
    /// for is not a subset. The *named* difference lives in
    /// [`Capabilities::missing_from`](super::super::missing) — this rule's
    /// message, not a second comparison.
    pub fn is_subset_of(&self, approved: &Capabilities) -> bool {
        if self.workspace_write && !approved.workspace_write {
            return false;
        }
        if self.scratch && !approved.scratch {
            return false;
        }
        if let Some(mine) = &self.fetch {
            let Some(theirs) = &approved.fetch else {
                return false;
            };
            if !mine
                .destinations
                .iter()
                .all(|d| theirs.destinations.contains(d))
            {
                return false;
            }
        }
        if let Some(mine) = &self.runner {
            let Some(theirs) = &approved.runner else {
                return false;
            };
            if !mine.programs.iter().all(|p| theirs.programs.contains(p)) {
                return false;
            }
        }
        self.endpoints.is_subset_of(&approved.endpoints)
    }
}
