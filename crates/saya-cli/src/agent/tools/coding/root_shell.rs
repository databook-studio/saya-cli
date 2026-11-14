//! The direct-mode root's `shell`: one command, run in the working checkout on
//! the host by default and, when the call asks for it and this host can prove
//! one, inside the same sandbox the workspace shell uses.
//!
//! The root advertises this tool whenever the mode is not `never`; `[tools]`
//! `allow` is the one switch that removes it. The tool itself — its schema, its lane
//! decision, and the composition that proves its sandbox — is
//! [`crate::agent::tools::shell_policy`], the one implementation every surface
//! that advertises `shell` builds from.

use std::{path::Path, sync::Arc, time::Instant};

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition, ToolError, ToolExecutionContext, ToolExecutor};
use tokio::sync::OnceCell;

use crate::commands::coding::NativeCodingJob;

use super::shell_secrets::host_policy;
use crate::agent::tools::consent::Caller;
use crate::agent::tools::shell_policy::{self, Background, ShellPlacement, ShellTool};

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "root_shell_hooks.rs"]
mod hooks;

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
pub(crate) use hooks::{build_host_only_with_spool, build_with_policy};

/// The root's shell: the shared tool, or the background composition that is
/// still preparing it.
pub(crate) struct RootShell {
    state: ShellState,
}

enum ShellState {
    /// The tool is composed and ready.
    Ready(ShellTool),
    /// The composition — which proves the sandbox and costs most of a second —
    /// runs on a blocking thread started at build. The first call waits for
    /// it; a job that never calls the shell pays nothing.
    Deferred {
        task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<Option<ShellTool>>>>,
        cell: OnceCell<Option<ShellTool>>,
    },
}

/// Builds the root shell, or `None` when it must not exist. `None` when the
/// mode is `never`, and `None` under `read-only` on a host where the sandbox
/// does not prove — a read-only session has no shell that could reach outside
/// it. Every sandbox refusal or error is a quiet `None` — "no shell", never an
/// error for the run — and nothing about the failure is logged as a secret.
///
/// Under every mode but `read-only` the composition always yields a shell (the
/// host lane when no sandbox proves), so the advertisement is fixed at build
/// and the probe runs in the background. Under `read-only` whether a shell
/// exists at all depends on the probe, so that composition stays synchronous
/// and the tool is advertised only when the sandbox proves.
pub(crate) fn build(
    job: &NativeCodingJob,
    checkout_root: &Path,
    read_witnesses: Arc<crate::agent::tools::workspace::ReadWitnesses>,
    witness_workspace: Option<Arc<saya_harness::workspace::Workspace>>,
) -> Option<RootShell> {
    let policy = job.approval_policy();
    if !shell_policy::advertised(policy) {
        return None;
    }
    // The operator, when one is present, is told which job is asking.
    let prompt = job.operator().map(|operator| {
        shell_policy::prompt(
            operator,
            Caller::Root {
                job_id: job.job_id().clone(),
            },
        )
    });
    let placement_root = checkout_root.to_path_buf();
    let spool = job.records().to_path_buf();
    let host = host_policy(job.runtime_config());
    let rules = job.runtime_config().resolved.permissions.rules.clone();
    let cancellation = job.cancellation();
    let lifecycle = job.lifecycle_arc();
    // The job's wall clock, taken once at build: every silent read waits no
    // longer than what remains of it.
    let wall_clock = crate::commands::coding::limits::wall_clock(&job.runtime_config().resolved)
        .map(shell_policy::WallClock::new);
    // The job's shell is the one shell that owns background runs: it lives as
    // long as the job, so a detached command has somewhere to be read from and
    // is killed and reaped when the job ends.
    let compose = move || {
        let started = Instant::now();
        let tool = shell_policy::compose(
            ShellPlacement::root(&placement_root).with_spool(&spool),
            policy,
            prompt,
            host,
            cancellation,
            Background::Owned,
            rules,
        )
        .map(|tool| match wall_clock {
            Some(clock) => tool.with_wall_clock(clock),
            None => tool,
        })
        .map(|tool| match witness_workspace.clone() {
            Some(workspace) => tool.with_read_witnesses(Arc::clone(&read_witnesses), workspace),
            None => tool,
        });
        // The probe is the expensive part; the span says what it cost without
        // holding the first model request for it.
        lifecycle.note_elapsed("shell_probe", started.elapsed());
        tool
    };
    if policy == saya_agent::ApprovalPolicy::ReadOnly {
        compose().map(RootShell::new)
    } else {
        Some(RootShell::deferred(compose))
    }
}

impl RootShell {
    /// Wraps the composed tool. Private to the construction seams, which are
    /// the only places a root shell is made.
    pub(super) fn new(tool: ShellTool) -> Self {
        Self {
            state: ShellState::Ready(tool),
        }
    }

    /// Starts the composition on a blocking thread and returns at once. The
    /// probe still runs on this host for this job; only when it is awaited
    /// changes.
    pub(super) fn deferred<F>(compose: F) -> Self
    where
        F: FnOnce() -> Option<ShellTool> + Send + 'static,
    {
        let task = tokio::task::spawn_blocking(compose);
        Self {
            state: ShellState::Deferred {
                task: tokio::sync::Mutex::new(Some(task)),
                cell: OnceCell::new(),
            },
        }
    }

    /// The composed tool, waiting for the background composition the first
    /// time. A composition that yielded no shell, or panicked, is `None`.
    async fn resolve(&self) -> Option<&ShellTool> {
        match &self.state {
            ShellState::Ready(tool) => Some(tool),
            ShellState::Deferred { task, cell } => cell
                .get_or_init(|| async {
                    let handle = task.lock().await.take();
                    match handle {
                        Some(handle) => handle.await.ok().flatten(),
                        None => None,
                    }
                })
                .await
                .as_ref(),
        }
    }

    /// Runs one approved call of the shell family: `shell` itself, or one of
    /// the two calls that reach a background run it started. The name decides
    /// which, through the schema's one table. A deferred composition is
    /// awaited here, before the first execution.
    pub(crate) async fn call(
        &self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let tool = self.resolve().await.ok_or_else(|| {
            ToolError::Runner("the shell could not be prepared on this host".to_owned())
        })?;
        tool.call(name, arguments).await
    }

    /// The background runs that ended since the loop last asked. A deferred
    /// composition that has not resolved yet holds no run, so it answers with
    /// nothing rather than waiting for the probe.
    pub(crate) fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        match &self.state {
            ShellState::Ready(tool) => tool.take_background_exits(),
            ShellState::Deferred { cell, .. } => cell
                .get()
                .and_then(Option::as_ref)
                .map(shell_policy::ShellTool::take_background_exits)
                .unwrap_or_default(),
        }
    }

    pub(crate) fn background_live(&self) -> bool {
        match &self.state {
            ShellState::Ready(tool) => tool.background_live(),
            ShellState::Deferred { cell, .. } => cell
                .get()
                .and_then(Option::as_ref)
                .is_some_and(shell_policy::ShellTool::background_live),
        }
    }

    pub(crate) fn background_notice_pending(&self) -> bool {
        match &self.state {
            ShellState::Ready(tool) => tool.background_notice_pending(),
            ShellState::Deferred { cell, .. } => cell
                .get()
                .and_then(Option::as_ref)
                .is_some_and(shell_policy::ShellTool::background_notice_pending),
        }
    }
}

#[async_trait]
impl ApprovalDecider for RootShell {
    /// The lane the shared policy names is allowed outright — a sandboxed call
    /// because the confinement is the consent, a host call under `bypass`
    /// because the mode said so — and only the host lane under `ask` reaches
    /// the operator.
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        match self.resolve().await {
            Some(shell) => shell.approve(tool, arguments).await,
            None => false,
        }
    }

    fn would_ask(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> Option<bool> {
        match &self.state {
            ShellState::Ready(shell) => shell.would_ask(tool, arguments),
            ShellState::Deferred { cell, .. } => cell
                .get()
                .and_then(Option::as_ref)
                .and_then(|shell| shell.would_ask(tool, arguments)),
        }
    }

    fn refusal_detail(
        &self,
        tool: &ToolDefinition,
        arguments: &serde_json::Value,
    ) -> Option<String> {
        match &self.state {
            ShellState::Ready(shell) => shell.refusal_detail(tool, arguments),
            ShellState::Deferred { cell, .. } => cell
                .get()
                .and_then(Option::as_ref)
                .and_then(|shell| shell.refusal_detail(tool, arguments)),
        }
    }
}

#[async_trait]
impl ToolExecutor for RootShell {
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        _context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.call(name, &arguments).await
    }

    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.call(name, &arguments).await
    }

    fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        RootShell::take_background_exits(self)
    }

    fn background_live(&self) -> bool {
        RootShell::background_live(self)
    }

    fn background_notice_pending(&self) -> bool {
        RootShell::background_notice_pending(self)
    }

    async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        match self.resolve().await {
            Some(tool) => tool.wait_background_exit(deadline, cancellation).await,
            None => false,
        }
    }
}
