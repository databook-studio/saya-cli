//! The runner's refusal battery: the typed errors, and the validation every
//! call passes before anything is spawned. Nothing is spawned past this
//! module — a refusal here is terminal, and the battery's ordering is most
//! specific first: name shape, interpreter refusal, the step's allowlist,
//! the timeout, then the program file itself.
//!
//! Four shapes are refused structurally, each one an escape the battery
//! names in CI:
//! - **paths** — a program is a bare name; the allowlist names programs and
//!   the sandbox allows exec inside one directory only;
//! - **shells and interpreters** — refused by name even when allowlisted;
//!   an interpreter can spawn arbitrary children with arbitrary argv and
//!   would void the typed-argv contract from inside the allowlist;
//! - **symlinks** — Seatbelt matches the resolved path, so a link out of
//!   the program directory cannot be honoured and must not look allowed;
//! - **scripts** — a shebang execs an interpreter that was never
//!   allowlisted; the wrapper trick dies here.

use std::{fs::File, io::Read as _, path::Path, time::Duration};

use saya_types::{InterpreterScope, RunnerScope, is_bare_name, is_refused_runner_program};

use super::error::RunnerError;

/// The runner's own reason for refusing a shell or interpreter by name. One
/// const, two doors: the runner's validation and the interpreter door's
/// fallback (a refused name the step's interpreter scope does not hold) both
/// render it, so the refusal cannot drift between them.
pub(super) const INTERPRETER_REFUSAL: &str = "shells and interpreters are refused by name: \
     an interpreter can spawn arbitrary children with arbitrary argv and would void the \
     typed-argv contract from inside the allowlist";

/// Maximum number of arguments in one typed runner call.
pub const MAX_ARG_COUNT: usize = 128;
/// Maximum size of one argument in bytes.
pub const MAX_ARG_BYTES: usize = 16 * 1024;
/// Maximum aggregate size of the argument vector in bytes.
pub const MAX_ARGV_BYTES: usize = 128 * 1024;

#[cfg(test)]
mod bounds_tests {
    use super::*;

    #[test]
    fn argv_count_item_and_total_bounds_refuse_before_spawn() {
        let allowed = RunnerScope::new(vec!["tool".to_owned()]).unwrap();
        let too_many = vec![String::new(); MAX_ARG_COUNT + 1];
        assert!(matches!(
            validate_call(
                &allowed,
                Path::new("/does/not/exist"),
                Duration::from_secs(5),
                None,
                "tool",
                &too_many,
            ),
            Err(RunnerError::ArgsNotTyped { .. })
        ));

        let too_long = vec!["x".repeat(MAX_ARG_BYTES + 1)];
        assert!(matches!(
            validate_call(
                &allowed,
                Path::new("/does/not/exist"),
                Duration::from_secs(5),
                None,
                "tool",
                &too_long,
            ),
            Err(RunnerError::ArgsNotTyped { .. })
        ));

        let item = "x".repeat(MAX_ARG_BYTES);
        let too_wide = vec![item; MAX_ARGV_BYTES / MAX_ARG_BYTES + 1];
        assert!(matches!(
            validate_call(
                &allowed,
                Path::new("/does/not/exist"),
                Duration::from_secs(5),
                None,
                "tool",
                &too_wide,
            ),
            Err(RunnerError::ArgsNotTyped { .. })
        ));
    }
}

/// How a validated call is spawned: everything the battery checked,
/// resolved to the only three things a child may receive.
#[derive(Debug)]
pub struct ValidatedCall {
    pub(super) program: String,
    pub(super) program_path: std::path::PathBuf,
    pub(super) argv: Vec<String>,
    pub(super) timeout: Duration,
}

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

fn inspect_head(path: &Path) -> std::io::Result<[u8; 2]> {
    let mut file = File::open(path)?;
    let mut head = [0u8; 2];
    match file.read_exact(&mut head) {
        Ok(()) => Ok(head),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok([0, 0]),
        Err(error) => Err(error),
    }
}

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

/// Validates the resource shape of typed argv before it is cloned into a
/// spawn request. Keeping this at the runner seam means direct callers cannot
/// bypass the CLI adapter's schema limits.
pub fn validate_argv(argv: &[String]) -> Result<(), RunnerError> {
    if argv.len() > MAX_ARG_COUNT {
        return Err(RunnerError::ArgsNotTyped {
            detail: "too many arguments",
        });
    }
    let mut total = 0usize;
    for arg in argv {
        if arg.len() > MAX_ARG_BYTES {
            return Err(RunnerError::ArgsNotTyped {
                detail: "an argument exceeds the per-argument byte limit",
            });
        }
        total = total.saturating_add(arg.len());
        if total > MAX_ARGV_BYTES {
            return Err(RunnerError::ArgsNotTyped {
                detail: "arguments exceed the aggregate byte limit",
            });
        }
    }
    Ok(())
}
