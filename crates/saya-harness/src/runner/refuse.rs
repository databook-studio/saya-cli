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

mod argv;
#[cfg(test)]
mod bounds_tests;
mod interpreter_battery;
mod runner_battery;

pub use argv::{MAX_ARG_BYTES, MAX_ARG_COUNT, MAX_ARGV_BYTES, validate_argv};
pub use interpreter_battery::validate_interpreter_call;
pub use runner_battery::validate_call;

use std::{fs::File, io::Read as _, path::Path, time::Duration};

/// The runner's own reason for refusing a shell or interpreter by name. One
/// const, two doors: the runner's validation and the interpreter door's
/// fallback (a refused name the step's interpreter scope does not hold) both
/// render it, so the refusal cannot drift between them.
pub(super) const INTERPRETER_REFUSAL: &str = "shells and interpreters are refused by name: \
     an interpreter can spawn arbitrary children with arbitrary argv and would void the \
     typed-argv contract from inside the allowlist";

/// How a validated call is spawned: everything the battery checked,
/// resolved to the only three things a child may receive.
#[derive(Debug)]
pub struct ValidatedCall {
    pub(super) program: String,
    pub(super) program_path: std::path::PathBuf,
    pub(super) argv: Vec<String>,
    pub(super) timeout: Duration,
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
