//! The `run_program` tool (M5-4): one allowlisted program, typed argv, no
//! shell, no interpolation — ever.
//!
//! [`RunProgram`] is the admitted tool, constructed around a proven
//! [`RunnerSpawn`] — the M5-3 startup probe's verdict, consumed here, not
//! re-derived — so an unproven host simply never gets this tool. The
//! allowlist is **per step**: the tool carries the plan-narrowed
//! [`RunnerScope`], a subset of the run's approved programs; approving the
//! plan approves the programs it names, and anything else is a typed
//! refusal in `refuse::validate` before anything is spawned. The child's
//! cwd is pinned to the run workspace by the spawn configuration; the
//! environment is built in `env`, empty by default, with only the declared
//! credentials injected. Timeout and cancellation kill the process group
//! (`spawn`); stdout and stderr are ring-buffered with byte caps and
//! redacted unconditionally — the resolved credential values themselves
//! scrubbed by the capture's value registry, then the `redact()` pattern
//! pass — before anything reaches the model or the run's disk record
//! (`output`).

pub mod env;
pub mod error;
pub mod nested_saya;
pub mod output;
pub mod refuse;
pub mod sandbox;
pub(crate) mod spawn;

mod call;
mod definition;
mod dispatch;
#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod dispatch_tests;

pub use env::{Credential, CredentialSource, SharedCredentialSource, StaticCredentialSource};
pub use error::RunnerError;
pub use output::{OUTPUT_CAP_BYTES, OutputRing, ProgramOutcome, StreamCapture};

use std::{path::PathBuf, time::Duration};

use saya_agent::CancellationToken;
use saya_types::{InterpreterScope, RunnerScope};

pub use refuse::{MAX_ARG_BYTES, MAX_ARG_COUNT, MAX_ARGV_BYTES, validate_argv};
use sandbox::RunnerSpawn;

/// The tool's name in the run engine's toolset.
pub const RUN_PROGRAM_TOOL: &str = "run_program";

/// The admitted `run_program` tool: one allowlisted program per call, under
/// the proven sandbox, with the step-narrowed allowlist and the declared
/// credentials.
///
/// The step's scopes open two doors on the same tool, and never one scope
/// wearing the other's name: the **runner door** admits the programs the
/// step's `RunnerScope` names; the **interpreter door** admits only the
/// names the runner refuses by name and the step's `InterpreterScope`
/// explicitly carries — the `--allow interpreter:<program>` grant, which
/// voids the typed-argv contract's behavioural half and is granted by the
/// typed token alone. A refused name outside the interpreter scope falls
/// back to the runner's byte-identical refusal, whatever doors the step
/// holds.
pub struct RunProgram {
    spawn: RunnerSpawn,
    /// The step's narrowed runner programs; `None` when the step asked for
    /// none — every non-refused name is then not allowlisted.
    runner: Option<RunnerScope>,
    /// The step's narrowed interpreter programs; `None` when the step asked
    /// for none — every refused name keeps the runner's byte-identical
    /// refusal.
    interpreters: Option<InterpreterScope>,
    credentials: Vec<Credential>,
    resolver: SharedCredentialSource,
    default_timeout: Duration,
    cancellation: CancellationToken,
    record_dir: PathBuf,
}

impl RunProgram {
    /// Builds the tool over a proven spawn, the step-narrowed allowlist, the
    /// configured default timeout, and the credential seam.
    pub fn new(
        spawn: RunnerSpawn,
        allowed: RunnerScope,
        default_timeout: Duration,
        resolver: SharedCredentialSource,
    ) -> Self {
        Self::for_step(spawn, Some(allowed), None, default_timeout, resolver)
    }

    /// Builds the tool from one step's capabilities: the runner door opens
    /// on the step's `RunnerScope`, the interpreter door on the step's
    /// `InterpreterScope` — each `None` when the step did not ask, so a
    /// step that did not ask never has the door at all.
    pub fn for_step(
        spawn: RunnerSpawn,
        runner: Option<RunnerScope>,
        interpreters: Option<InterpreterScope>,
        default_timeout: Duration,
        resolver: SharedCredentialSource,
    ) -> Self {
        Self {
            record_dir: spawn.fs_roots()[0].join("run_program"),
            spawn,
            runner,
            interpreters,
            credentials: Vec::new(),
            resolver,
            default_timeout,
            cancellation: CancellationToken::new(),
        }
    }

    /// Attaches the credentials the approved plan declared for this step:
    /// only what is on this list can ever reach a child's environment.
    pub fn with_credentials(mut self, credentials: Vec<Credential>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Attaches the run's cancellation token: a cancelled run kills the
    /// child's whole process group and reports the cancellation.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Redirects the outcome record out of the default (`fs_roots[0]` /
    /// `run_program`). The interactive session uses this: its root is the
    /// user's project, which gains no saya-created directories, so the
    /// record lands in the session's own state directory instead.
    pub fn with_record_dir(mut self, dir: PathBuf) -> Self {
        self.record_dir = dir;
        self
    }
}

fn not_typed(detail: &'static str) -> RunnerError {
    RunnerError::ArgsNotTyped { detail }
}

fn validate_json_argv(items: &[serde_json::Value]) -> Result<(), RunnerError> {
    if items.len() > MAX_ARG_COUNT {
        return Err(not_typed("too many arguments"));
    }
    let mut total = 0usize;
    for item in items {
        let value = item
            .as_str()
            .ok_or_else(|| not_typed("every element of args must be a string"))?;
        if value.len() > MAX_ARG_BYTES {
            return Err(not_typed("an argument exceeds the per-argument byte limit"));
        }
        total = total.saturating_add(value.len());
        if total > MAX_ARGV_BYTES {
            return Err(not_typed("arguments exceed the aggregate byte limit"));
        }
    }
    Ok(())
}
