//! The session's `run_program` composition, once per session process: the
//! sandbox over the one workspace root with empty egress, the placement
//! guard, then the startup probe. A refused probe leaves no `RunnerSpawn`,
//! so the tool is absent for the whole session — no grant, no approval,
//! nothing can conjure a capability the composition could not construct
//! (UNIFY §6's rule, restated as a session invariant).
//!
//! The interpreter door stays shut in this slice: nothing can produce the
//! typed token that grants an interpreter (U2's session grants), so the
//! family keeps the runner's byte-identical refusal. Bypass, grants, and the
//! tri-choice are later slices; the probe gates everything regardless of
//! any future mode.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use saya_harness::runner::sandbox::{RunSandbox, RunnerSpawn};
use saya_harness::runner::{RunProgram, SharedCredentialSource, StaticCredentialSource};

use super::session_definitions;
use crate::commands::run::runner::place_guard;

/// The session's runner wiring: the proven spawn (one per session), the
/// config's runner allow as the per-call door (the ask is the per-call
/// consent), and the resolved timeout ceiling.
pub(crate) struct SessionRunner {
    pub(super) spawn: RunnerSpawn,
    pub(super) scope: saya_types::RunnerScope,
    pub(super) timeout: Duration,
    pub(super) record_dir: PathBuf,
    /// The tool definition the session advertises, built once at composition
    /// from the proven spawn's own declaration, then reworded for the
    /// session surface.
    pub(super) definition: saya_agent::ToolDefinition,
}

/// The session runner's composition: the sandbox over the one workspace
/// root with empty egress, the placement guard (which now runs against the
/// project — a checked-in tool directory is inside the roots and refuses),
/// then the startup probe. A config allowlist with no `program_dir`, or an
/// empty `allow`, composes nothing: a capability that gates nothing is not
/// constructed.
pub(super) fn compose_runner(
    runtime: &crate::config::runtime::RuntimeConfig,
    workspace_root: &Path,
    state_dir: &Path,
) -> Result<Option<SessionRunner>, String> {
    let jobs = &runtime.resolved.jobs.runner;
    let Some(program_dir) = jobs.program_dir.as_ref() else {
        return Ok(None);
    };
    if jobs.allow.is_empty() {
        return Ok(None);
    }
    let sandbox = RunSandbox::new([workspace_root.to_path_buf()], Vec::<(String, u16)>::new())
        .map_err(|error| format!("the session's sandbox policy could not be composed: {error}"))?;
    let program_dir = place_session(program_dir, sandbox.fs_roots())?;
    let provision = sandbox
        .prepare(&program_dir)
        .map_err(|error| format!("the runner sandbox could not be prepared: {error}"))?;
    let Some(spawn) = provision.spawn() else {
        // Probe refused: the tool is absent for the session — no grant, no
        // approval can conjure a capability the composition could not
        // construct.
        return Ok(None);
    };
    let scope = saya_types::RunnerScope::new(jobs.allow.clone())
        .map_err(|error| format!("resolved [jobs.runner] allow is not a usable scope: {error}"))?;
    let timeout = std::time::Duration::from_secs(jobs.timeout_seconds);
    let record_dir = state_dir.join("run_program");
    // The tool's own definition, built once at composition from the proven
    // spawn (its effect reads the declared egress — empty here), then
    // reworded and ask-gated for the session surface.
    let resolver: SharedCredentialSource = Arc::new(StaticCredentialSource::new(Vec::new()));
    let definition = session_definitions::run_program(
        RunProgram::for_step(spawn.clone(), Some(scope.clone()), None, timeout, resolver)
            .definition(),
    );
    Ok(Some(SessionRunner {
        spawn: spawn.clone(),
        scope,
        timeout,
        record_dir,
        definition,
    }))
}

/// The placement guard, in the session's own words. The guard is not
/// relaxed for sessions: the default recommended layout (the platform data
/// home's program directory) is disjoint and fine, but a program directory
/// inside the workspace root — a project's checked-in tool directory — has
/// a containment relation to a session's only fs root, and the enforcement
/// cannot express an exclusion: Seatbelt subpaths are allow-lists and
/// Landlock has no subtractive rights.
fn place_session(program_dir: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    place_guard(
        program_dir,
        roots,
        |dir| {
            format!(
                "the runner program directory {} could not be resolved — set [jobs.runner] \
                 program_dir to an existing directory and stage its programs there",
                dir.display()
            )
        },
        |canonical, root| {
            format!(
                "the runner program directory {} overlaps this session's workspace root {} — \
                 a program directory inside, equal to, or containing the session's workspace \
                 root lets one session child write the binary the next run_program call \
                 validates green and executes, and the enforcement cannot express an \
                 exclusion (Seatbelt subpaths are allow-lists and Landlock has no subtractive \
                 rights), so a checked-in tool directory cannot be used from a session; \
                 stage programs outside the workspace root",
                canonical.display(),
                root.display()
            )
        },
    )
}
