//! The runner battery: the validation every `run_program` call passes
//! before anything is spawned — name shape, the interpreter refusal, the
//! step's allowlist, the timeout, then the program file itself.

use std::path::Path;
use std::time::Duration;

use saya_types::{RunnerScope, is_bare_name, is_refused_runner_program};

use crate::runner::error::RunnerError;

use super::{INTERPRETER_REFUSAL, ValidatedCall, inspect_head, validate_argv};

/// Validates one call before anything is spawned.
pub fn validate_call(
    allowed: &RunnerScope,
    program_dir: &Path,
    default_timeout: Duration,
    requested_timeout: Option<u64>,
    program: &str,
    argv: &[String],
) -> Result<ValidatedCall, RunnerError> {
    validate_argv(argv)?;
    if !is_bare_name(program) {
        return Err(RunnerError::ProgramRefused {
            program: program.to_owned(),
            reason: "a program is a bare name, never a path or traversal — the allowlist \
                     names programs and the sandbox allows exec inside one directory only",
        });
    }
    if is_refused_runner_program(program) {
        return Err(RunnerError::ProgramRefused {
            program: program.to_owned(),
            reason: INTERPRETER_REFUSAL,
        });
    }
    if !allowed.programs.iter().any(|allowed| allowed == program) {
        return Err(RunnerError::ProgramNotAllowlisted {
            program: program.to_owned(),
        });
    }
    // The configured default is the ceiling; a call may narrow it, never
    // widen it, and zero is a typo, not an instant pause.
    let timeout = match requested_timeout {
        Some(0) => return Err(RunnerError::TimeoutNotPositive),
        Some(secs) if Duration::from_secs(secs) > default_timeout => {
            return Err(RunnerError::TimeoutExceedsDefault {
                requested: secs,
                default: default_timeout.as_secs(),
            });
        }
        Some(secs) => Duration::from_secs(secs),
        None => default_timeout,
    };
    let program_path = program_dir.join(program);
    let metadata =
        std::fs::symlink_metadata(&program_path).map_err(|_| RunnerError::ProgramMissing {
            program: program.to_owned(),
            path: program_path.display().to_string(),
        })?;
    if metadata.file_type().is_symlink() {
        return Err(RunnerError::ProgramRefused {
            program: program.to_owned(),
            reason: "the allowlisted program is a symlink — Seatbelt matches the resolved \
                     path, so a link out of the program directory cannot be honoured",
        });
    }
    if !metadata.is_file() {
        return Err(RunnerError::ProgramRefused {
            program: program.to_owned(),
            reason: "the allowlisted program is not a regular file",
        });
    }
    // A script's shebang execs its own interpreter — the wrapper trick. The
    // first two bytes decide; a file shorter than a shebang is not one.
    let head = inspect_head(&program_path).map_err(|source| RunnerError::ProgramFile {
        path: program_path.display().to_string(),
        source,
    })?;
    if head == *b"#!" {
        return Err(RunnerError::ProgramRefused {
            program: program.to_owned(),
            reason: "the allowlisted program is a script — its shebang would exec an \
                     interpreter that was never allowlisted",
        });
    }
    Ok(ValidatedCall {
        program: program.to_owned(),
        program_path,
        argv: argv.to_vec(),
        timeout,
    })
}
