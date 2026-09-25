//! The host executor: one PATH-resolved program, typed argv, a built
//! environment, a stated timeout. No caller yet — this slice builds the
//! executor only.
//!
//! Mechanics: the name resolves against the PATH value the child receives
//! (`resolve`), the child's environment is built from that PATH plus the
//! caller's explicit variables (`env`), and the child runs under the stated
//! timeout with the shared process-group kill (`crate::proc`) and the
//! shared output cap and redaction (`runner::output`). There is no staging
//! directory and no admission battery here — both exist to keep exec inside
//! one directory, and this lane has no such directory — so a PATH-resolved
//! symlink or script is honoured, not refused.

pub mod env;
pub mod resolve;

pub use resolve::{HostConfig, HostError};

use std::process::Stdio;

use saya_agent::CancellationToken;

use super::runner::output::ProgramOutcome;

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

/// One host call: a bare program name plus typed argv. Every element of
/// `argv` is passed verbatim as one argv element — no shell, no
/// interpolation, no command-line string anywhere.
#[derive(Debug, Clone)]
pub struct HostCommand {
    program: String,
    argv: Vec<String>,
}

impl HostCommand {
    /// Builds one call. The program must be a bare name — resolved against
    /// the PATH the child receives, never a path or traversal.
    pub fn new(
        program: impl Into<String>,
        argv: impl IntoIterator<Item = String>,
    ) -> Result<Self, HostError> {
        let program = program.into();
        if !saya_types::is_bare_name(&program) {
            return Err(HostError::NameNotBare { program });
        }
        Ok(Self {
            program,
            argv: argv.into_iter().collect(),
        })
    }

    /// Runs the call: resolves the name against the config's PATH, builds
    /// the child's environment, and spawns under the stated timeout with the
    /// config's workspace root as the child's cwd. The
    /// config's timeout is the ceiling — a call may narrow it, never widen
    /// it, and zero is refused. Cancellation kills the whole process group
    /// and is reported.
    pub async fn run(
        &self,
        config: &HostConfig,
        timeout_seconds: Option<u64>,
        cancellation: &CancellationToken,
    ) -> Result<ProgramOutcome, HostError> {
        let path = config.resolve(&self.program)?;
        let timeout = match timeout_seconds {
            None => config.timeout(),
            Some(secs) => config.narrow_timeout(secs)?,
        };
        let mut command = std::process::Command::new(&path);
        // Typed argv, verbatim: every element is one argv element. There is
        // no string here that becomes a command line.
        command.args(&self.argv);
        // The child's cwd pins to the workspace root: the prompt's `cwd:
        // pinned to <root>` fact holds because this line applies it.
        command.current_dir(config.workspace_root());
        config.apply_env(&mut command);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command
            .spawn()
            .map_err(|source| HostError::Spawn { source })?;
        let pid = child.id();
        let pipes = crate::proc::Pipes::take(&mut child.stdout, &mut child.stderr)
            .map_err(|source| HostError::WaitFailed { source })?;
        let settled = crate::proc::wait(child, timeout, cancellation)
            .await
            .map_err(|source| HostError::WaitFailed { source })?;
        // No secrets ride the child's environment beyond what the caller
        // passed through, so the capture gate redacts with an empty
        // registry plus the unconditional pattern pass.
        let (stdout, stderr) = pipes.finish(&[]);
        Ok(crate::proc::outcome(
            self.program.clone(),
            pid,
            &settled,
            stdout,
            stderr,
        ))
    }
}
