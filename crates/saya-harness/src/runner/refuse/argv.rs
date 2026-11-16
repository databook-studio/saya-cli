//! The typed-argv resource bounds every process call passes before spawn.

/// Maximum number of arguments in one typed runner call.
pub const MAX_ARG_COUNT: usize = 128;
/// Maximum size of one argument in bytes.
pub const MAX_ARG_BYTES: usize = 16 * 1024;
/// Maximum aggregate size of the argument vector in bytes.
pub const MAX_ARGV_BYTES: usize = 128 * 1024;
