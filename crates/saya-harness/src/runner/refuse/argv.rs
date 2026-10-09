//! The typed-argv resource bounds every runner call passes before spawn.

use crate::runner::error::RunnerError;

/// Maximum number of arguments in one typed runner call.
pub const MAX_ARG_COUNT: usize = 128;
/// Maximum size of one argument in bytes.
pub const MAX_ARG_BYTES: usize = 16 * 1024;
/// Maximum aggregate size of the argument vector in bytes.
pub const MAX_ARGV_BYTES: usize = 128 * 1024;

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
