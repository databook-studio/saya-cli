//! The call's whole validation, and the two shapes of child environment: the
//! command's bounds, the timeout's bounds, and the workdir resolved exactly
//! once — before anything spawns.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use saya_types::WorkspacePath;

use super::{
    DEFAULT_TIMEOUT, MAX_COMMAND_BYTES, MAX_STDIN_BYTES, MAX_TIMEOUT, SHELL_PATH, SHELL_PROGRAM,
    WorkspaceShell, refused,
};
use crate::runner::RunnerError;

#[cfg(test)]
#[path = "validate_tests.rs"]
mod validate_tests;

/// How a validated call is spawned: everything the shell checked, resolved
/// to the only four things a child may receive. The shell is its only
/// constructor (`WorkspaceShell::validated_call`); `spawn::execute` is its
/// only consumer.
#[derive(Debug)]
pub(crate) struct ValidatedCall {
    pub(crate) program: String,
    pub(crate) program_path: PathBuf,
    pub(crate) argv: Vec<String>,
    pub(crate) timeout: Duration,
    /// The bytes the child reads on stdin, already inside
    /// [`MAX_STDIN_BYTES`]. `None` leaves the child's stdin at EOF.
    pub(crate) stdin: Option<Vec<u8>>,
}

impl WorkspaceShell {
    /// The call's whole validation, and the only place a `ValidatedCall` is
    /// built on this path: the command's shape, the timeout's bounds, and the
    /// workdir resolved exactly once.
    pub(super) fn validated_call(
        &self,
        call: &super::call::ShellCall,
    ) -> Result<(ValidatedCall, PathBuf), RunnerError> {
        if call.command.is_empty() {
            return Err(refused("the command must not be empty"));
        }
        if call.command.len() > MAX_COMMAND_BYTES {
            return Err(refused(format!(
                "the command is {} bytes; the ceiling is {MAX_COMMAND_BYTES}",
                call.command.len()
            )));
        }
        if call.command.contains('\0') {
            return Err(refused("the command must not contain a NUL byte"));
        }
        let timeout = match call.timeout {
            None => DEFAULT_TIMEOUT,
            Some(timeout) if timeout.is_zero() => {
                return Err(refused("the timeout must be at least one millisecond"));
            }
            Some(timeout) if timeout > MAX_TIMEOUT => {
                return Err(refused(format!(
                    "the requested timeout {}s exceeds the {}s ceiling",
                    timeout.as_secs(),
                    MAX_TIMEOUT.as_secs()
                )));
            }
            Some(timeout) => timeout,
        };
        let stdin = match call.input.as_ref() {
            None => None,
            Some(input) if input.len() > MAX_STDIN_BYTES => {
                return Err(refused(format!(
                    "the stdin payload is {} bytes; the ceiling is {MAX_STDIN_BYTES}",
                    input.len()
                )));
            }
            Some(input) => Some(input.clone()),
        };
        let cwd = self.resolve_workdir(call.workdir.as_ref())?;
        Ok((
            ValidatedCall {
                program: SHELL_PROGRAM.to_owned(),
                program_path: PathBuf::from(SHELL_PROGRAM),
                argv: vec!["-c".to_owned(), call.command.clone()],
                timeout,
                stdin,
            },
            cwd,
        ))
    }

    /// Resolves the call's workdir to the exact path the child will start in.
    /// The join happens once, the canonical form is what is checked and what
    /// is used: a symlink that leaves the workspace resolves outside the root
    /// and is refused here, before anything spawns.
    fn resolve_workdir(&self, workdir: Option<&WorkspacePath>) -> Result<PathBuf, RunnerError> {
        let Some(workdir) = workdir else {
            return Ok(self.root.clone());
        };
        let resolved = fs::canonicalize(self.root.join(workdir.as_str())).map_err(|_| {
            refused(format!(
                "the workdir `{}` does not resolve inside the workspace",
                workdir.as_str()
            ))
        })?;
        if !resolved.starts_with(&self.root) {
            return Err(refused(format!(
                "the workdir `{}` resolves outside the workspace root",
                workdir.as_str()
            )));
        }
        if !resolved.is_dir() {
            return Err(refused(format!(
                "the workdir `{}` is not a directory",
                workdir.as_str()
            )));
        }
        Ok(resolved)
    }
}

/// The sandboxed path's whole environment: `HOME` is the workspace root, so
/// a child that reads it reads the workspace and nothing else. A host child
/// gets none of this — it inherits the caller's own environment — so the
/// pairs are built for the path that uses them and no other.
pub(super) fn shell_env(root: &Path, toolchain: &[PathBuf]) -> Vec<(&'static str, OsString)> {
    shell_env_with_scratch(root, None, toolchain)
}

/// The same environment over a caller-owned scratch home, with `TMPDIR`
/// beside it: the shape a helper's shell gives its sandboxed child. The
/// toolchain directories lead `PATH`, so a sandboxed build finds the host's
/// compilers without the checkout naming them.
pub(super) fn shell_env_with_scratch(
    home: &Path,
    tmpdir: Option<&Path>,
    toolchain: &[PathBuf],
) -> Vec<(&'static str, OsString)> {
    let mut env = vec![
        (
            "PATH",
            super::toolchain::path_with_toolchain(toolchain, SHELL_PATH),
        ),
        ("HOME", home.as_os_str().to_owned()),
        ("LANG", OsString::from("C.UTF-8")),
        ("TERM", OsString::from("dumb")),
    ];
    if let Some(tmpdir) = tmpdir {
        env.insert(2, ("TMPDIR", tmpdir.as_os_str().to_owned()));
    }
    env
}
