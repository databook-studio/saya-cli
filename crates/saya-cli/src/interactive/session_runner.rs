//! The session's `run_program` composition, once per session process: the
//! sandbox over the one workspace root with empty egress, the placement
//! guard, then the startup probe. A refused probe leaves no `RunnerSpawn`,
//! so the tool is absent for the whole session — no grant, no approval,
//! nothing can conjure a capability the composition could not construct
//! (UNIFY §6's rule, restated as a session invariant).
//!
//! Two doors open on the one proven spawn, exactly like a run's step: the
//! **runner door** on the resolved `[jobs.runner] allow`, and the
//! **interpreter door** on the resolved `[jobs.interpreter] allow` — the
//! trusted config's staged universe, built at composition, mode-independently
//! (capability in the composition, consent in the approval engine — the same
//! shape as the runner door). An interpreter call still passes the per-call
//! battery (`validate_interpreter_call`: bare name, scope membership,
//! timeout, a real non-symlink non-script binary), unstaged names keep the
//! byte-identical family refusal, and the probe gates every door: on an
//! unproven host there is no runner at all, in any mode.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use saya_harness::runner::sandbox::{RunSandbox, RunnerSpawn};
use saya_harness::runner::{RunProgram, SharedCredentialSource, StaticCredentialSource};

use super::session_definitions;
use crate::commands::run::runner::place_guard;

/// The probe-refused fact, said on the composition notice seam (DESIGN §2):
/// today's shape returns `Ok(None)` silently, but a user who asked for
/// "everything runs" — or staged programs and got nothing — must hear what
/// cannot. The same notice serves every mode; the bypass activation line
/// references it.
pub(crate) const PROBE_REFUSED_NOTICE: &str =
    "run_program is unavailable: the sandbox probe did not prove this host.";

/// The session's runner wiring: the proven spawn (one per session), the
/// config's runner allow as the per-call runner door, the config's
/// interpreter allow as the interpreter door, and the resolved timeout
/// ceiling.
pub(crate) struct SessionRunner {
    pub(super) spawn: RunnerSpawn,
    pub(super) scope: saya_types::RunnerScope,
    /// The interpreter door: `Some` when the trusted config staged
    /// interpreters in `[jobs.interpreter] allow`, `None` when none are —
    /// no staged universe, no door. Mode-independent: the ask (or bypass)
    /// is the consent, this scope is what the consent can reach.
    pub(super) interpreters: Option<saya_types::InterpreterScope>,
    pub(super) timeout: Duration,
    pub(super) record_dir: PathBuf,
    /// The tool definition the session advertises, built once at composition
    /// from the proven spawn's own declaration, then reworded for the
    /// session surface.
    pub(super) definition: saya_agent::ToolDefinition,
}

/// What composing the session runner produced: the wiring for the executor,
/// and the probe's verdict when it refused — a fact the session must say,
/// never a silent absence.
pub(super) struct RunnerComposition {
    pub(super) runner: Option<SessionRunner>,
    /// The probe-refused notice, `Some` exactly when the host did not prove.
    pub(super) probe_notice: Option<String>,
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
) -> Result<RunnerComposition, String> {
    let jobs = &runtime.resolved.jobs.runner;
    let Some(program_dir) = jobs.program_dir.as_ref() else {
        return Ok(RunnerComposition {
            runner: None,
            probe_notice: None,
        });
    };
    if jobs.allow.is_empty() {
        return Ok(RunnerComposition {
            runner: None,
            probe_notice: None,
        });
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
        // construct — and the absence is said, on the notice seam.
        return Ok(RunnerComposition {
            runner: None,
            probe_notice: Some(PROBE_REFUSED_NOTICE.to_owned()),
        });
    };
    let scope = saya_types::RunnerScope::new(jobs.allow.clone())
        .map_err(|error| format!("resolved [jobs.runner] allow is not a usable scope: {error}"))?;
    // The interpreter door's universe: the trusted config's staged
    // interpreters. `None` when nothing is staged — an empty `[jobs.interpreter]
    // allow` is no capability, not a closed door on one that exists. The
    // shape was validated at config resolve (`jobs.rs`); a failure here is a
    // contract break, so it refuses the composition rather than opening a
    // half-built door.
    let interpreters = if runtime.resolved.jobs.interpreter.allow.is_empty() {
        None
    } else {
        Some(
            saya_types::InterpreterScope::new(runtime.resolved.jobs.interpreter.allow.clone())
                .map_err(|error| {
                    format!("resolved [jobs.interpreter] allow is not a usable scope: {error}")
                })?,
        )
    };
    let timeout = std::time::Duration::from_secs(jobs.timeout_seconds);
    let record_dir = state_dir.join("run_program");
    // The tool's own definition, built once at composition from the proven
    // spawn (its effect reads the declared egress — empty here), then
    // reworded and ask-gated for the session surface.
    let resolver: SharedCredentialSource = Arc::new(StaticCredentialSource::new(Vec::new()));
    let definition = session_definitions::run_program(
        RunProgram::for_step(
            spawn.clone(),
            Some(scope.clone()),
            interpreters.clone(),
            timeout,
            resolver,
        )
        .definition(),
    );
    Ok(RunnerComposition {
        runner: Some(SessionRunner {
            spawn: spawn.clone(),
            scope,
            interpreters,
            timeout,
            record_dir,
            definition,
        }),
        probe_notice: None,
    })
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
