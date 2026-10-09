//! The runner's wiring at the composition root: the placement guard, the
//! startup probe, and the admission check — the two composition-side checks
//! that make the escape report's contract mechanical (S3-PROGRAM-DIR.md).
//!
//! `RunSandbox::prepare` canonicalises the program directory and probes this
//! host, but no layer inside the runner compares the directory against the
//! run's filesystem roots, and `refuse::validate_call` cannot detect a
//! binary a previous child wrote over a sibling: a rewritten file is a
//! regular, non-symlink file and passes every per-call check. The
//! composition root therefore runs two checks of its own, in order, before
//! any plan is proposed:
//!
//! - **The placement guard** — the canonical program dir must have no
//!   path-containment relation, in either direction, to any fs root: not
//!   inside one, not equal to one, not containing one. With programs inside
//!   the run tree, one step's child writes the binary the next step's
//!   `run_program` validates green and executes.
//! - **The admission check**, after the proven arm and before `propose` —
//!   every program the run approved must appear in resolved
//!   `[jobs.runner] allow`, must not name a refused interpreter, and must
//!   resolve in the directory to a regular, non-symlink, non-script file:
//!   the same battery `refuse::validate_call` applies per call, applied
//!   once, at start. A failure refuses the run (exit class 3) naming the
//!   program, the directory, and the reason.

use std::path::{Path, PathBuf};
use std::time::Duration;

mod admission;
#[cfg(test)]
pub(super) use admission::{admit, admit_interpreters};

use saya_config::{ResolvedInterpreterJobs, ResolvedRunnerJobs};
use saya_harness::runner::sandbox::{RunSandbox, RunnerSpawn};
use saya_types::Capabilities;

/// The run's runner wiring, built once per run when the run approved a
/// runner scope and the startup probe proved the host. The step toolsets
/// build their `run_program` members from it; cloning the spawn grants
/// nothing — the proven arm of `prepare` remains the only source.
#[derive(Debug)]
pub(super) struct RunRunner {
    pub(super) spawn: RunnerSpawn,
    /// The resolved `[jobs.runner] timeout_seconds` — the configured
    /// default ceiling each step's tool narrows, never widens.
    pub(super) timeout: Duration,
}

/// What building the runner wiring produced: the wiring for the toolsets,
/// and the capabilities plan validation must see — the run's approved scopes
/// passed through the provision's own fail-closed rule, so the runner scope
/// survives only where the probe proved the host.
#[derive(Debug)]
pub(super) struct RunnerWiring {
    pub(super) runner: Option<RunRunner>,
    pub(super) plan_scopes: Capabilities,
}

/// Builds the run's runner wiring, exactly once per run, fresh and resume
/// alike. A run that approved neither a runner nor an interpreter scope
/// never consults the directory: no check, no failure, the tool simply
/// absent from every toolset — so `saya ask` and any read-only run are
/// unaffected by any state of `[jobs.runner]` or `[jobs.interpreter]`. One
/// program directory serves both families, so one probe covers both; the
/// admission check runs per family, against that family's universe.
pub(super) fn build(
    jobs: &ResolvedRunnerJobs,
    interpreters: &ResolvedInterpreterJobs,
    run_root: &Path,
    scopes: &Capabilities,
) -> Result<RunnerWiring, String> {
    if scopes.runner.is_none() && scopes.interpreter.is_none() {
        return Ok(RunnerWiring {
            runner: None,
            plan_scopes: scopes.clone(),
        });
    }
    let Some(program_dir) = jobs.program_dir.clone() else {
        return Err(
            "the run approved a runner or interpreter scope but [jobs.runner] program_dir is \
             not set: stage the approved programs and interpreters in one directory and set \
             program_dir to its absolute path"
                .to_string(),
        );
    };
    // The fs roots are the run workspace first, then the run's state/ —
    // workspace first because the proven spawn pins the child's cwd to the
    // first root (the cwd-pinning rule) — and no child egress: nothing
    // configures runner egress, and the empty set is the fail-closed
    // composition. The host-allowlisted egress a plan may need is the fetch
    // policy's, in-process.
    let sandbox = RunSandbox::new(
        [run_root.join("workspace"), run_root.join("state")],
        Vec::<(String, u16)>::new(),
    )
    .map_err(|error| format!("the run's sandbox policy could not be composed: {error}"))?;
    // The placement guard, before anything else consults the directory.
    let program_dir = place(&program_dir, sandbox.fs_roots())?;
    // The entry point, once per run: canonicalise, then the startup probe —
    // the host and the policy, again on every resume.
    let provision = sandbox
        .prepare(&program_dir)
        .map_err(|error| format!("the runner sandbox could not be prepared: {error}"))?;
    // The capabilities plan validation sees: the provision's own fail-closed
    // rule. On a refused probe the runner scope is stripped, so a plan
    // asking for the runner refuses as needs-approval — the capability is
    // absent, never degraded.
    let plan_scopes = provision.plan_capabilities(scopes);
    let runner = match provision.spawn() {
        Some(spawn) => {
            // The admission check, per family against that family's
            // universe: explicitly stated programs that cannot run refuse
            // the run at start, never a tool whose every call refuses
            // mid-flight.
            if let Some(approved) = scopes.runner.as_ref() {
                admission::admit(
                    approved,
                    &jobs.allow,
                    spawn.program_dir(),
                    Duration::from_secs(jobs.timeout_seconds),
                )?;
            }
            if let Some(approved_interpreters) = scopes.interpreter.as_ref() {
                admission::admit_interpreters(
                    approved_interpreters,
                    &interpreters.allow,
                    spawn.program_dir(),
                    Duration::from_secs(jobs.timeout_seconds),
                )?;
            }
            Some(RunRunner {
                spawn: spawn.clone(),
                timeout: Duration::from_secs(jobs.timeout_seconds),
            })
        }
        None => None,
    };
    Ok(RunnerWiring {
        runner,
        plan_scopes,
    })
}

/// The placement guard: the canonical program dir must not sit inside, equal
/// to, or containing any fs root. Both containment directions are checked —
/// a program dir containing a root puts a child-writable root under the exec
/// allow, the measured escape one composition mistake away. The mechanics
/// are the one implementation both surfaces share; the refusal text is each
/// surface's own contract, supplied here in the run's words.
pub(super) fn place(program_dir: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
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
                "the runner program directory {} overlaps this run's filesystem root {} — \
                 a program directory inside, equal to, or containing a run root lets one \
                 step's child write the binary the next step's run_program validates green \
                 and executes; stage programs outside the run tree",
                canonical.display(),
                root.display()
            )
        },
    )
}

/// The shared guard mechanics: canonicalise the program directory (it must
/// resolve to an existing directory), then refuse any containment relation —
/// in either direction — to any root. Each surface renders its own refusal.
pub(crate) fn place_guard(
    program_dir: &Path,
    roots: &[PathBuf],
    unresolvable: impl Fn(&Path) -> String,
    overlap: impl Fn(&Path, &Path) -> String,
) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(program_dir).map_err(|_| unresolvable(program_dir))?;
    if !canonical.is_dir() {
        return Err(unresolvable(program_dir));
    }
    for root in roots {
        if canonical.starts_with(root) || root.starts_with(&canonical) {
            return Err(overlap(&canonical, root));
        }
    }
    Ok(canonical)
}
