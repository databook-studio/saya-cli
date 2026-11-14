//! The engine: one hook, run through the same bounded path a `shell` call
//! takes, and the three lifecycle points that call it.
//!
//! # What a hook can and cannot do
//!
//! A `PreToolUse` hook's exit code is the **only** decision any hook makes:
//! `2` blocks the call and hands the hook's own bounded, redacted stderr back
//! to the model as the reason; every other exit — `0`, a signal, a timeout, a
//! command that could not be spawned — allows the call and reports a
//! diagnostic. `PostToolUse` and `Stop` decide nothing at all: they run, and
//! whatever they print is theirs.
//!
//! No hook can grant a tool, widen a scope, change an approval mode, or answer
//! an ask. The engine sits *inside* every surface's own gate — after the
//! `[tools]` allow list, after the plan posture, after the approval decision —
//! so a hook's exit 0 can only ever let through a call the surface had already
//! allowed. A hook that exits 0 for a tool the surface removed changes
//! nothing: the surface still refuses it.
//!
//! # The lane
//!
//! A hook is the user's own command, stated in a protected config layer a
//! repository cannot write, so it runs on the **host** lane — the lane where
//! the user's `jq`, `cargo` and `git` are. It goes through the same
//! [`WorkspaceShell`] every `shell` call goes through: the same bounded
//! capture, the same redaction pass, the same process-group kill on timeout
//! and cancellation. A surface composes no engine at all where its mode keeps
//! no host lane (`read-only`, `never`), so a read-only session runs no hook.
//!
//! # Bounds
//!
//! Every one is named: hooks per event (`saya-config`'s
//! [`MAX_HOOKS_PER_EVENT`](saya_config::MAX_HOOKS_PER_EVENT)), the command's
//! bytes, the per-hook timeout, the payload's bytes ([`payload`]), the
//! reason's bytes ([`MAX_REASON_BYTES`]), and the session-start context's
//! bytes ([`MAX_SESSION_CONTEXT_BYTES`]). A hook that hangs is killed and
//! reaped by the harness's own timeout; a hook that fails is a diagnostic. The
//! agent loop is never wedged by either.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolError};
use saya_config::{HookEvent, ResolvedHook, ResolvedHooks};
use saya_harness::runner::{HostPolicy, ShellCall, WorkspaceShell};

use super::payload;
use crate::config::runtime::RuntimeConfig;

#[path = "runner_lifecycle.rs"]
mod lifecycle;

/// The longest reason a blocking hook may hand the model. The hook's own
/// stderr, bounded with an elision marker, so a script that dumps its whole
/// log cannot spend the turn's context on a refusal.
pub(super) const MAX_REASON_BYTES: usize = 1024;

/// The longest context a `SessionStart` hook may add to the model's turn.
/// It is the same bound the module already gives a hook's words, so no hook
/// can spend more of the context than a refusal can.
pub(super) const MAX_SESSION_CONTEXT_BYTES: usize = MAX_REASON_BYTES;

/// Who a hook is running for: the identity its input carries, so one script
/// can tell a coding job's calls from its helpers', a `saya run`'s, or a
/// session's.
#[derive(Debug, Clone)]
pub(crate) enum HookCaller {
    /// A coding job's root, by id.
    Job(String),
    /// One of a coding job's helpers: the job it belongs to and the agent id
    /// the job gave it. A helper's calls are the job's calls, so a hook that
    /// blocks one blocks it for the helper too — and the script can still tell
    /// which helper made it.
    Helper { agent_id: String, job_id: String },
    /// A `saya run`, by id. Every step of the run is the same caller: the
    /// hooks cover the run, not one step of it.
    Run(String),
    /// An interactive session, by record id where it has one. A universe
    /// with no acquisition behind it has none, and a hook then reads no id
    /// rather than a made-up one.
    Session(Option<String>),
    /// The one-shot `ask` path, which owns no session record.
    Ask,
}

/// The hooks one surface runs, over the workspace they run in.
pub(crate) struct Hooks {
    hooks: ResolvedHooks,
    shell: WorkspaceShell,
    /// The root and host policy the shell was composed over. Kept so
    /// `session_end` can run after the session's own cancellation has fired:
    /// a hook that observes the end must not be cancelled by the end.
    root: PathBuf,
    host: HostPolicy,
    caller: HookCaller,
}

impl Hooks {
    /// Builds the engine over `root`. A host-only shell: no sandbox is proven
    /// or needed, because a hook is the user's own command.
    pub(crate) fn new(
        hooks: ResolvedHooks,
        root: &Path,
        host: HostPolicy,
        caller: HookCaller,
        cancellation: CancellationToken,
    ) -> Result<Self, saya_harness::runner::RunnerError> {
        Ok(Self {
            hooks,
            shell: WorkspaceShell::host_only(root.to_path_buf(), host.clone())?
                .with_cancellation(cancellation),
            root: root.to_path_buf(),
            host,
            caller,
        })
    }

    /// The engine a surface composes, or `None` where it must not have one: no
    /// hook is configured, or the mode keeps no host lane. `read-only` and
    /// `never` are the two modes where a `shell` call cannot reach the host
    /// either, so a hook has no lane to run in and none is composed.
    pub(crate) fn compose(
        runtime: &RuntimeConfig,
        mode: ApprovalPolicy,
        root: &Path,
        caller: HookCaller,
        cancellation: CancellationToken,
    ) -> Option<Arc<Self>> {
        if !permits(mode) || runtime.resolved.hooks.is_empty() {
            return None;
        }
        let host = crate::agent::tools::shell_policy::host_policy(runtime);
        match Self::new(
            runtime.resolved.hooks.clone(),
            root,
            host,
            caller,
            cancellation,
        ) {
            Ok(hooks) => Some(Arc::new(hooks)),
            Err(error) => {
                // A root that does not resolve is the caller's fact, not the
                // user's: say so once, run no hook, and leave the run alone.
                eprintln!("warning: hooks are configured but cannot run here: {error}");
                None
            }
        }
    }

    /// Runs this call's `PreToolUse` hooks. `Err` means one of them blocked the
    /// call; the error carries that hook's bounded stderr as the reason.
    pub(crate) async fn pre_tool_use(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let mut input: Option<String> = None;
        for hook in self.watching(HookEvent::PreToolUse, tool) {
            let payload = input.get_or_insert_with(|| {
                payload::tool_event(&self.caller, HookEvent::PreToolUse, tool, arguments, None)
            });
            match self.run(hook, payload).await {
                HookRun::Exited {
                    code: Some(2),
                    stderr,
                    ..
                } => {
                    return Err(ToolError::NoEffect(Box::new(ToolError::HookBlocked(
                        stderr,
                    ))));
                }
                HookRun::Exited { code: Some(0), .. } => {}
                other => self.diagnostic(HookEvent::PreToolUse, other),
            }
        }
        Ok(())
    }

    /// Runs this call's `PostToolUse` hooks. Nothing they do changes the call:
    /// the outcome has already been decided and returned to the model.
    pub(crate) async fn post_tool_use(
        &self,
        tool: &str,
        arguments: &serde_json::Value,
        succeeded: bool,
    ) {
        let mut input: Option<String> = None;
        for hook in self.watching(HookEvent::PostToolUse, tool) {
            let payload = input.get_or_insert_with(|| {
                payload::tool_event(
                    &self.caller,
                    HookEvent::PostToolUse,
                    tool,
                    arguments,
                    Some(succeeded),
                )
            });
            let run = self.run(hook, payload).await;
            if !matches!(run, HookRun::Exited { code: Some(0), .. }) {
                self.diagnostic(HookEvent::PostToolUse, run);
            }
        }
    }

    /// Runs the `Stop` hooks: the agent has finished this turn. Nothing they
    /// do changes the turn's outcome.
    pub(crate) async fn stop(&self) {
        let hooks = self.hooks.for_event(HookEvent::Stop);
        if hooks.is_empty() {
            return;
        }
        let payload = payload::stop_event(&self.caller);
        for hook in hooks {
            let run = self.run(hook, &payload).await;
            if !matches!(run, HookRun::Exited { code: Some(0), .. }) {
                self.diagnostic(HookEvent::Stop, run);
            }
        }
    }

    /// The hooks of `event` that watch `tool`, in declaration order.
    fn watching(&self, event: HookEvent, tool: &str) -> impl Iterator<Item = &ResolvedHook> {
        self.hooks
            .for_event(event)
            .iter()
            .filter(move |hook| hook.watches(tool))
    }

    /// Runs one hook through the shell: the same bounded capture, redaction
    /// pass, timeout kill and process-group reap a `shell` call gets.
    async fn run(&self, hook: &ResolvedHook, input: &str) -> HookRun {
        Self::run_in(&self.shell, hook, input).await
    }

    /// Runs one hook over a shell composed without this session's
    /// cancellation. The session is ending; its end hook must still run.
    async fn run_uncancelled(&self, hook: &ResolvedHook, input: &str) -> HookRun {
        match WorkspaceShell::host_only(self.root.clone(), self.host.clone()) {
            Ok(shell) => Self::run_in(&shell, hook, input).await,
            Err(error) => HookRun::Refused(error.to_string()),
        }
    }

    async fn run_in(shell: &WorkspaceShell, hook: &ResolvedHook, input: &str) -> HookRun {
        let call = ShellCall::new(hook.command().command_line())
            .with_sandbox(false)
            .with_timeout(hook.timeout())
            .with_input(input.as_bytes().to_vec());
        match shell.run(call).await {
            Ok(outcome) if outcome.cancelled => HookRun::Cancelled,
            Ok(outcome) if outcome.timed_out => HookRun::TimedOut(hook.timeout()),
            Ok(outcome) => HookRun::Exited {
                code: outcome.exit_code,
                stdout: saya_agent::bound_tool_transcript(
                    &outcome.stdout.text,
                    MAX_SESSION_CONTEXT_BYTES,
                )
                .trim()
                .to_owned(),
                stderr: saya_agent::bound_tool_transcript(&outcome.stderr.text, MAX_REASON_BYTES)
                    .trim()
                    .to_owned(),
            },
            Err(error) => HookRun::Refused(error.to_string()),
        }
    }

    /// Reports a hook that did not exit 0 on an event where that is not a
    /// decision. One bounded line on stderr, naming the event and the reason —
    /// a failing hook must be visible without being able to stop the run.
    fn diagnostic(&self, event: HookEvent, run: HookRun) {
        eprintln!(
            "warning: {} hook did not run cleanly: {}",
            event.as_str(),
            run.describe()
        );
    }
}

/// Whether a mode keeps the host lane a hook runs in. The asking modes (`ask`,
/// `accept-edits`) and `bypass` do — a hook is the user's own command, and the
/// asking modes reach it through the operator; `read-only`'s only lane is the
/// sandbox, where a hook could change nothing it was meant to change, and
/// `never` keeps no lane at all.
pub(crate) fn permits(mode: ApprovalPolicy) -> bool {
    matches!(
        mode,
        ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits | ApprovalPolicy::Bypass
    )
}

/// What one hook run reported. The variant is the decision, so no caller has
/// to re-read an exit code to find out what happened.
enum HookRun {
    /// The hook exited. `code` is `None` when a signal ended it. `stdout` is
    /// bounded context (only `SessionStart` offers it to the model); `stderr`
    /// is a possible refusal reason.
    Exited {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    /// The hook's timeout fired; the harness killed and reaped its whole
    /// process group. The call it was watching proceeds.
    TimedOut(Duration),
    /// The run was cancelled with the surface.
    Cancelled,
    /// The shell refused the call before anything spawned.
    Refused(String),
}

impl HookRun {
    /// The one line a diagnostic says. A blocking hook's own words are never
    /// here: this is the report for a hook that did *not* decide anything.
    fn describe(&self) -> String {
        match self {
            HookRun::Exited {
                code,
                stderr,
                stdout: _,
            } if stderr.is_empty() => {
                format!("exited {}", describe_code(*code))
            }
            HookRun::Exited { code, stderr, .. } => {
                format!("exited {}: {stderr}", describe_code(*code))
            }
            HookRun::TimedOut(timeout) => format!(
                "killed after its {} ms timeout; the call it was watching was not blocked",
                timeout.as_millis()
            ),
            HookRun::Cancelled => "cancelled with the run".to_owned(),
            HookRun::Refused(detail) => format!("could not be started: {detail}"),
        }
    }
}

fn describe_code(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("with code {code}"),
        None => "on a signal".to_owned(),
    }
}
