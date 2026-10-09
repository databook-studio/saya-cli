use std::path::Path;
use std::time::Duration;

use saya_harness::runner::RunnerError;
use saya_harness::runner::refuse::{validate_call, validate_interpreter_call};
use saya_types::{InterpreterScope, RunnerScope, is_refused_runner_program};

/// The admission check: every program in the run's approved runner scope —
/// the superset of every step's narrowed scope — must be one the runner can
/// actually run. The battery's ordering mirrors `refuse::validate_call`'s:
/// most specific first, the interpreter refusal before the allowlist, so a
/// name the runner will never honour gets its own reason.
pub(in crate::commands::run) fn admit(
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
pub(in crate::commands::run) fn admit_interpreters(
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
