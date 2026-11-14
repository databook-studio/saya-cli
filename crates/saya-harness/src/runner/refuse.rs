//! The typed-argv resource bounds every process call passes before spawn.
//!
//! The workspace shell resolves its own calls — the command's shape, the
//! timeout's bounds, the workdir — in [`super::shell::WorkspaceShell`], and
//! only the argv bounds here gate the host `run_command` path.

mod argv;

pub use argv::{MAX_ARG_BYTES, MAX_ARG_COUNT, MAX_ARGV_BYTES};
