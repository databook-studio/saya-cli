//! The interpreter door: validation for a program the runner refuses by
//! name but the run's approved interpreter scope explicitly carries — the
//! same structural gates, in the battery's order.

use std::path::Path;
use std::time::Duration;

use saya_types::{InterpreterScope, is_bare_name};

use crate::runner::error::RunnerError;

use super::{ValidatedCall, inspect_head, validate_argv};

/// Validates one interpreter call — a program the runner refuses by name but
/// the run's approved interpreter scope explicitly carries. Every structural
/// gate still applies, in the battery's order: the bare-name rule first (a
/// path is a refusal at every layer), the scope's own allowlist, the
/// timeout, then the program file itself — symlink, non-regular-file, and
/// shebang refusals included, so an approved interpreter must still be a
/// staged real binary in the one program directory. The interpreter refusal
/// itself does not fire here: this door exists only for names the runner
/// refuses, and the caller consults it only when the scope holds the name —
/// a refused name outside the scope falls back to the runner's
/// byte-identical refusal (`validate_call`).
pub fn validate_interpreter_call(
    allowed: &InterpreterScope,
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
    if !allowed.contains(program) {
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
