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

use saya_config::{ResolvedInterpreterJobs, ResolvedRunnerJobs};
use saya_harness::runner::RunnerError;
use saya_harness::runner::refuse::validate_call;
use saya_harness::runner::refuse::validate_interpreter_call;
use saya_harness::runner::sandbox::{RunSandbox, RunnerSpawn};
use saya_types::{Capabilities, InterpreterScope, RunnerScope, is_refused_runner_program};

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
                admit(
                    approved,
                    &jobs.allow,
                    spawn.program_dir(),
                    Duration::from_secs(jobs.timeout_seconds),
                )?;
            }
            if let Some(approved_interpreters) = scopes.interpreter.as_ref() {
                admit_interpreters(
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
/// allow, the measured escape one composition mistake away.
pub(super) fn place(program_dir: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let unresolvable = || {
        format!(
            "the runner program directory {} could not be resolved — set [jobs.runner] \
             program_dir to an existing directory and stage its programs there",
            program_dir.display()
        )
    };
    let canonical = std::fs::canonicalize(program_dir).map_err(|_| unresolvable())?;
    if !canonical.is_dir() {
        return Err(unresolvable());
    }
    for root in roots {
        if canonical.starts_with(root) || root.starts_with(&canonical) {
            return Err(format!(
                "the runner program directory {} overlaps this run's filesystem root {} — \
                 a program directory inside, equal to, or containing a run root lets one \
                 step's child write the binary the next step's run_program validates green \
                 and executes; stage programs outside the run tree",
                canonical.display(),
                root.display()
            ));
        }
    }
    Ok(canonical)
}

/// The admission check: every program in the run's approved runner scope —
/// the superset of every step's narrowed scope — must be one the runner can
/// actually run. The battery's ordering mirrors `refuse::validate_call`'s:
/// most specific first, the interpreter refusal before the allowlist, so a
/// name the runner will never honour gets its own reason.
pub(super) fn admit(
    scope: &RunnerScope,
    allow: &[String],
    program_dir: &Path,
    default_timeout: Duration,
) -> Result<(), String> {
    for program in &scope.programs {
        if is_refused_runner_program(program) {
            return Err(admission_refusal(
                program,
                program_dir,
                "shells and interpreters are refused: an interpreter can spawn arbitrary \
                 children with arbitrary argv and would void the typed-argv contract from \
                 inside the allowlist",
            ));
        }
        if !allow.iter().any(|allowed| allowed == program) {
            return Err(admission_refusal(
                program,
                program_dir,
                "it is not declared in [jobs.runner] allow — a run's runner scope may name \
                 only programs the config allows",
            ));
        }
        // The same battery `validate_call` applies per call, once at start:
        // the file must exist as a regular, non-symlink, non-script file.
        let allowed =
            RunnerScope::new(vec![program.clone()]).expect("the scope's entry is a bare name");
        match validate_call(&allowed, program_dir, default_timeout, None, program, &[]) {
            Ok(_) => {}
            Err(RunnerError::ProgramMissing { .. }) => {
                return Err(format!(
                    "runner program {program:?} approved by --allow is not in {}: stage a \
                     regular, non-script, non-symlink file with that name",
                    program_dir.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "runner program {program:?} approved by --allow is not usable in {}: {error}. \
                 Stage a regular, non-script, non-symlink file there.",
                    program_dir.display()
                ));
            }
        }
    }
    Ok(())
}

fn admission_refusal(program: &str, program_dir: &Path, reason: &str) -> String {
    format!(
        "runner program {program:?} approved by --allow is not usable in {}: {reason}",
        program_dir.display()
    )
}

/// The interpreter family's admission check, mirroring `admit`: every
/// program in the run's approved interpreter scope must be one the resolved
/// `[jobs.interpreter] allow` universe declared, and must stage as a real,
/// non-symlink, non-script file in the one program directory — the same
/// battery `validate_interpreter_call` applies per call, applied once, at
/// start. An approved interpreter whose bytes are absent refuses the run
/// rather than granting a name that answers with nothing.
pub(super) fn admit_interpreters(
    scope: &InterpreterScope,
    allow: &[String],
    program_dir: &Path,
    default_timeout: Duration,
) -> Result<(), String> {
    for program in &scope.programs {
        if !allow.iter().any(|allowed| allowed == program) {
            return Err(interpreter_admission_refusal(
                program,
                program_dir,
                "it is not declared in [jobs.interpreter] allow — a run's interpreter scope \
                 may name only interpreters the trusted config allows",
            ));
        }
        let allowed =
            InterpreterScope::new(vec![program.clone()]).expect("the scope's entry is a bare name");
        match validate_interpreter_call(&allowed, program_dir, default_timeout, None, program, &[])
        {
            Ok(_) => {}
            Err(RunnerError::ProgramMissing { .. }) => {
                return Err(format!(
                    "interpreter {program:?} approved by --allow is not staged in {}: stage a \
                     real, non-script, non-symlink interpreter binary with that name there",
                    program_dir.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "interpreter {program:?} approved by --allow is not usable in {}: {error}. \
                     Stage a real, non-script, non-symlink interpreter binary there.",
                    program_dir.display()
                ));
            }
        }
    }
    Ok(())
}

fn interpreter_admission_refusal(program: &str, program_dir: &Path, reason: &str) -> String {
    format!(
        "interpreter {program:?} approved by --allow is not usable in {}: {reason}",
        program_dir.display()
    )
}
