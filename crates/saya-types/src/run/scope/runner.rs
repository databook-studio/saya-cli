//! The runner capability: the programs a run's plan may narrow a step to,
//! and the shells and interpreters no allowlist may ever name.

use serde::{Deserialize, Serialize};

use super::naming::is_bare_name;
use crate::run::RunContractError;

pub const MAX_RUNNER_PROGRAMS: usize = 32;

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
