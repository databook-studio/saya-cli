// The Windows system variables a child whose environment was cleared keeps.
//
// `Command::env_clear` gives the child nothing, which is what makes these e2e
// rows hermetic — and on Windows leaves it without `SystemRoot`, so the process
// cannot initialise Winsock and its first HTTP request fails before anything
// reaches the network. The scripted-provider child reported that only as
// "provider recovery: not retried attempt 1 of 9", and every line-front row
// timed out on real Windows.
//
// Both cleared-environment harnesses include this file rather than keeping a
// list of their own: the managed-job child (`tests/job_apply_e2e/support.rs`)
// and the scripted-provider child
// (`tests/support/scripted_provider/spawned_child.rs`). Unix needs none of
// them, so the file is a no-op there and nothing else in the child's
// environment changes.
//
// `include!` splices items, so this file states its context as plain comments:
// an inner `//!` header would be invalid after the including module's items.

/// The Windows variables a child needs to load and to start ordinary programs.
/// Without `SystemRoot` a process cannot initialise Winsock, so its first HTTP
/// request fails before anything reaches the network. Unix has no equivalent.
#[cfg(windows)]
const WINDOWS_SYSTEM_ENV: &[&str] = &[
    "SystemRoot",
    "WINDIR",
    "SystemDrive",
    "TEMP",
    "TMP",
    "ComSpec",
    "PATHEXT",
    "LOCALAPPDATA",
    "APPDATA",
    "ProgramData",
];

/// Re-adds the Windows system variables from the test process to a command
/// whose environment was cleared. A no-op on unix, which needs none of them.
#[cfg(windows)]
pub(crate) fn inherit_windows_system_env(command: &mut std::process::Command) {
    for name in WINDOWS_SYSTEM_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn inherit_windows_system_env(_command: &mut std::process::Command) {}
