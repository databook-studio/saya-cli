//! Proving the sandbox and composing the shell: which lanes the mode allows,
//! and the one place a shell is built.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_config::CommandRules;
use saya_harness::runner::sandbox::{RunSandbox, RunnerSpawn, SandboxProvision};
use saya_harness::runner::{
    BackgroundRuns, HostPolicy, SHELL_PATH, SHELL_SPOOL_CAPTURE_CAP_BYTES, WorkspaceShell,
};

use super::model_output::OutputSpool;
use super::placement::ShellPlacement;
use super::policy::ShellPolicy;
use super::tool::ShellTool;
use crate::agent::tools::consent::Prompt;

/// Whether the shell being composed owns the background family, and where its
/// runs live. The decision travels with the lanes because it is the same kind
/// of decision: a shell that does not outlive a call must not offer a run that
/// outlives one.
pub(crate) enum Background {
    /// The surface's shell lives for one call or one step: no background
    /// family, and a `run_in_background` flag is refused.
    Off,
    /// The shell lives as long as the surface that composed it (a job's root,
    /// a helper): its registry and its cancellation token end with it.
    Owned,
    /// The shell is composed per turn, but its runs belong to a longer-lived
    /// owner (a session): every turn's shell reaches the same registry, and
    /// the owner's token ends the runs.
    Shared {
        runs: Arc<BackgroundRuns>,
        cancellation: CancellationToken,
    },
}

impl Background {
    /// Whether this composition advertises and serves the background family.
    fn enabled(&self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Attaches the shared registry, where there is one. `Off` and `Owned`
    /// leave the shell with the registry it built for itself.
    fn attach(self, shell: WorkspaceShell) -> WorkspaceShell {
        match self {
            Self::Off | Self::Owned => shell,
            Self::Shared { runs, cancellation } => shell.with_shared_background(runs, cancellation),
        }
    }
}

/// The measured reason `process-fork` is granted for: a shell forks for every
/// pipeline and every background job.
const FORK_REASON: &str = "the root's workspace shell runs pipelines and background jobs, which \
     fork - measured deny without the allow: /bin/sh: fork: Operation not permitted - spike 4.5";

/// Whether a surface advertises `shell` at all: every mode but `never` does.
/// The `[tools] allow` list is the one switch that removes it, and it narrows
/// after composition, so a composed shell is advertised here.
pub(crate) fn advertised(policy: ApprovalPolicy) -> bool {
    policy != ApprovalPolicy::Never
}

/// Proves the sandbox over the placement's root and builds the shell, choosing
/// the lanes the mode allows. `None` where the config is off or the mode is
/// `never`, and `None` under `read-only` on a host where the sandbox does not
/// prove — a read-only session has no shell that could reach outside it. Every
/// sandbox refusal or error is a quiet `None` — "no shell", never an error for
/// the caller — and nothing about the failure is logged as a secret.
///
/// `background` is the caller's decision about the background family, taken
/// with the lanes because it is the same kind of decision: the surface whose
/// shell outlives one call — a coding job's root, a helper — owns it, a
/// session shares one registry across its per-turn shells, and a shell that
/// lives for one call offers none, so no shell offers a run it could not keep.
pub(crate) fn compose(
    placement: ShellPlacement<'_>,
    policy: ApprovalPolicy,
    prompt: Option<Prompt>,
    host: HostPolicy,
    cancellation: CancellationToken,
    background: Background,
    rules: CommandRules,
) -> Option<ShellTool> {
    match prove(placement.root, policy) {
        Some(spawn) => Some(compose_sandboxed(
            placement,
            policy,
            spawn,
            prompt,
            host,
            cancellation,
            background,
            rules,
        )),
        // No proven sandbox: the host lane alone is all this host has, and a
        // read-only session does not take it.
        None if policy == ApprovalPolicy::ReadOnly => None,
        None => compose_host_only(
            placement.root,
            policy,
            prompt,
            host,
            cancellation,
            background,
            placement.spool,
            rules,
        ),
    }
}

/// The shell over a proven spawn: the sandbox lane always, and the host lane
/// too unless the mode is `read-only`. A helper's scratch tree is pinned here,
/// so its sandboxed lane's `HOME`/`TMPDIR` are its own.
#[allow(clippy::too_many_arguments)] // each argument is a distinct, named input; grouping would obscure the call sites
pub(crate) fn compose_sandboxed(
    placement: ShellPlacement<'_>,
    policy: ApprovalPolicy,
    spawn: RunnerSpawn,
    prompt: Option<Prompt>,
    host: HostPolicy,
    cancellation: CancellationToken,
    background: Background,
    rules: CommandRules,
) -> ShellTool {
    let enabled = background.enabled();
    let mut shell = WorkspaceShell::new(spawn)
        .expect("the proven spawn is confined to one workspace with no egress")
        .with_cancellation(cancellation);
    let spool = placement
        .spool
        .and_then(|dir| OutputSpool::new(dir, placement.root));
    if spool.is_some() {
        shell = shell.with_capture_cap(SHELL_SPOOL_CAPTURE_CAP_BYTES);
    }
    if let Some((home, tmpdir)) = placement.scratch {
        shell = shell.with_scratch(home.to_path_buf(), tmpdir.to_path_buf());
    }
    let shell = if policy == ApprovalPolicy::ReadOnly {
        shell
    } else {
        shell.with_host(host)
    };
    let shell = background.attach(shell).with_timeout_detach(enabled);
    ShellTool::new(
        shell,
        ShellPolicy::new(
            policy,
            prompt,
            true,
            rules,
            Some(placement.root.to_path_buf()),
        ),
        enabled,
        spool.map(Arc::new),
    )
}

/// The shell over the host lane alone, for a host with no proven sandbox. The
/// host lane inherits this process's environment, so a helper's scratch tree
/// does not apply to it: what a host run is, is the machine.
#[allow(clippy::too_many_arguments)] // each argument is a distinct, named input; grouping would obscure the call sites
pub(crate) fn compose_host_only(
    root: &Path,
    policy: ApprovalPolicy,
    prompt: Option<Prompt>,
    host: HostPolicy,
    cancellation: CancellationToken,
    background: Background,
    spool: Option<&Path>,
    rules: CommandRules,
) -> Option<ShellTool> {
    let enabled = background.enabled();
    let mut shell = WorkspaceShell::host_only(root.to_path_buf(), host)
        .ok()?
        .with_cancellation(cancellation);
    let spool = spool.and_then(|dir| OutputSpool::new(dir, root));
    if spool.is_some() {
        shell = shell.with_capture_cap(SHELL_SPOOL_CAPTURE_CAP_BYTES);
    }
    let shell = background.attach(shell).with_timeout_detach(enabled);
    Some(ShellTool::new(
        shell,
        ShellPolicy::new(policy, prompt, false, rules, Some(root.to_path_buf())),
        enabled,
        spool.map(Arc::new),
    ))
}

/// Proves the sandbox over `root`. Under `read-only` the whole root is
/// write-denied, so even a sandboxed command can change nothing; otherwise
/// nothing is denied, and the command may write its whole root — `.git`
/// included.
pub(crate) fn prove(root: &Path, policy: ApprovalPolicy) -> Option<RunnerSpawn> {
    let dirs: Vec<PathBuf> = SHELL_PATH.split(':').map(PathBuf::from).collect();
    let sandbox = RunSandbox::new([root.to_path_buf()], Vec::<(String, u16)>::new())
        .ok()?
        // The host's toolchain directories are readable and executable, never
        // writable: a sandboxed `cargo test` reaches the compiler, and the
        // checkout stays the only writable tree.
        .with_read_only_roots(saya_harness::runner::toolchain_dirs_for_process())
        .ok()?
        .with_process_fork(FORK_REASON)
        .ok()?;
    // `read-only` is the one mode that still needs a write denial: the whole
    // root, so a sandboxed command can read and run but change nothing. Every
    // other mode denies nothing — `.git` is as writable as the rest.
    let sandbox = if policy == ApprovalPolicy::ReadOnly {
        sandbox.with_write_denied(root.to_path_buf()).ok()?
    } else {
        sandbox
    };
    match sandbox.prepare_with_program_dirs(&dirs).ok()? {
        SandboxProvision::Proven { spawn, .. } => Some(spawn),
        SandboxProvision::Refused(_) => None,
    }
}
