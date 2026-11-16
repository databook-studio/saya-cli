//! The shell itself: the tool, and the lane decision that says which call it
//! may serve. The routing that hands a `shell` call to that decision rather
//! than to a surface's coarse effect gate is the shared seam
//! ([`crate::agent::tools::consent::OwnedToolApproval`]).

use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition, ToolError, ToolExecutionContext, ToolExecutor};
use saya_harness::runner::{ShellCall, StreamCapture, WorkspaceShell};
use saya_types::WorkspacePath;
use serde_json::{Value, json};

use super::arguments::validate;
use super::model_output::{self, OutputSpool};
use super::policy::{Lane, ShellPolicy};
use super::schema::{Operation, operation};
use crate::agent::tools::consent::OwnApproval;

/// The shell itself: the composed lanes and the policy that governs them.
/// Both `execute` and the lane decision live here, so a surface that holds
/// one holds the other's answer.
pub(crate) struct ShellTool {
    pub(super) shell: WorkspaceShell,
    policy: ShellPolicy,
    /// Whether this shell advertises — and serves — the background family.
    /// Composed where the shell is, because it is the same decision as the
    /// lanes: a surface whose shell does not outlive a call must not offer a
    /// run that outlives one.
    pub(super) background: bool,
    /// Where a result too large for the model's inline bound saves its full
    /// captured output. `None` on a surface with no job state directory; the
    /// result is still bounded, and its marker says the full output was not
    /// saved.
    pub(super) spool: Option<std::sync::Arc<OutputSpool>>,
    /// The consecutive silent reads of every run this shell started, so a
    /// model that polls anyway is answered with bounded waits, not a stream
    /// of instant requests.
    pub(super) reads: super::poll::ReadBook,
    /// The job's wall clock, where the surface has one: every silent-read
    /// wait is clamped to what remains of it.
    pub(super) wall_clock: Option<super::poll::WallClock>,
    /// The run's read-witness store and workspace root. A successful
    /// single-file view records the file's digest there, so a later `edit`
    /// with no `expected_digest` is admitted as if it followed a `read`.
    pub(super) read_witness: Option<super::view::ShellReadWitness>,
}

impl ShellTool {
    /// Wraps the composed lanes and the policy that governs them. Private to
    /// this module's composition seam, which is the only place a shell is made.
    pub(super) fn new(
        shell: WorkspaceShell,
        policy: ShellPolicy,
        background: bool,
        spool: Option<std::sync::Arc<OutputSpool>>,
    ) -> Self {
        Self {
            shell,
            policy,
            background,
            spool,
            reads: super::poll::ReadBook::default(),
            wall_clock: None,
            read_witness: None,
        }
    }

    /// Joins this shell to the run's read-witness store: a successful
    /// single-file view (`cat`, `head`, `tail`, `sed -n`, `nl`) then guards a
    /// later edit without a digest.
    pub(in crate::agent::tools) fn with_read_witnesses(
        mut self,
        witnesses: std::sync::Arc<crate::agent::tools::workspace::ReadWitnesses>,
        workspace: std::sync::Arc<saya_harness::workspace::Workspace>,
    ) -> Self {
        self.read_witness = Some(super::view::ShellReadWitness {
            witnesses,
            workspace,
        });
        self
    }

    /// Clamps every silent-read wait to the job's remaining wall clock.
    pub(crate) fn with_wall_clock(mut self, clock: super::poll::WallClock) -> Self {
        self.wall_clock = Some(clock);
        self
    }

    /// The test seam for the silent-read backoff: a table of short waits so a
    /// battery does not spend a real 30 s on every read.
    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    pub(crate) fn with_silent_waits(mut self, waits: [Duration; 3]) -> Self {
        self.reads = super::poll::ReadBook::new(waits);
        self
    }

    /// The background runs that ended since the loop last asked, when this
    /// shell advertises the background family at all.
    pub(crate) fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        if self.background {
            self.shell.take_background_exits()
        } else {
            Vec::new()
        }
    }

    /// Whether any background run this shell holds is still going.
    pub(crate) fn background_live(&self) -> bool {
        self.background && self.shell.background_live()
    }

    /// Whether a background run ended and its notice has not been delivered.
    pub(crate) fn background_notice_pending(&self) -> bool {
        self.background && self.shell.background_notice_pending()
    }

    /// Waits for the next background exit without issuing a model request.
    pub(crate) async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        self.background
            && self
                .shell
                .wait_background_exit(deadline, cancellation)
                .await
    }

    /// Bounds one result for the model, saving the full captured output under
    /// the job's state directory where the result does not fit. Every
    /// model-facing shell result passes through here — the foreground run and
    /// the background read alike — so the 30,000-character ceiling cannot be
    /// missed by one path.
    pub(super) fn bound(
        &self,
        stdout: &StreamCapture,
        stderr: &StreamCapture,
    ) -> model_output::Bounded {
        match &self.spool {
            Some(spool) => spool.bound(stdout, stderr),
            None => model_output::bound_without_spool(stdout, stderr),
        }
    }

    /// The family's one entry: the name decides which of its three calls runs.
    /// A name that is not the shell's is unsupported here — the surfaces that
    /// route the family route exactly [`operation`]'s names.
    pub(crate) async fn call(&self, name: &str, arguments: &Value) -> Result<Value, ToolError> {
        match operation(name) {
            Some(Operation::Run) => self.run_call(arguments).await,
            Some(Operation::Output) => self.output_call(arguments).await,
            Some(Operation::Kill) => self.kill_call(arguments).await,
            None => Err(ToolError::UnsupportedTool),
        }
    }

    /// Runs one approved call — waited for, or started in the background. A
    /// run that started and was cancelled is a [`ToolError::Runner`], never a
    /// `NoEffect`: the child did run. Named apart from the
    /// [`ToolExecutor::execute`] this type also implements, which carries the
    /// tool name this dispatches on.
    pub(crate) async fn run_call(&self, arguments: &Value) -> Result<Value, ToolError> {
        validate(arguments, self.background)?;
        let command = arguments
            .get("command")
            .and_then(Value::as_str)
            .ok_or(ToolError::UnsupportedProperty)?;
        let (sandboxed, lane_notice) = match self.policy.lane(arguments) {
            Lane::Sandbox(notice) => (true, notice),
            Lane::Host(_) => (false, None),
            // `approve` refuses this call before `execute` is reached; one
            // that arrives here anyway is refused with the same reason.
            Lane::Refused(reason) => return Err(ToolError::Runner(reason.to_owned())),
        };
        let mut call = ShellCall::new(command).with_sandbox(sandboxed);
        if let Some(workdir) = arguments
            .get("workdir")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
        {
            let path = WorkspacePath::try_from(workdir.to_owned())
                .map_err(|_| ToolError::PathNotString)?;
            call = call.with_workdir(path);
        }
        if let Some(millis) = arguments.get("timeout").and_then(Value::as_u64) {
            call = call.with_timeout(Duration::from_millis(millis));
        }
        if arguments.get("run_in_background") == Some(&Value::Bool(true)) {
            let id = self
                .shell
                .start_background(call.with_background(true))
                .await
                .map_err(|error| ToolError::Runner(error.to_string()))?;
            let mut started = json!({
                "id": id,
                "status": "running",
                "stdout": "",
                "stderr": "",
                "sandboxed": sandboxed,
            });
            // The lane a start took is the lane a foreground call would take,
            // notice and all: a detached command rerouted to the sandbox says
            // so where the model already reads where its command ran.
            if let Some(notice) = lane_notice {
                started["lane_notice"] = Value::String(notice.to_owned());
            }
            return Ok(started);
        }
        let outcome = self
            .shell
            .run(call)
            .await
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        if outcome.cancelled {
            return Err(ToolError::Runner(
                "the root workspace shell was cancelled".into(),
            ));
        }
        // A foreground call that outlived its timeout is still running: the
        // model reads its id here and reaches it with `shell_output`, exactly
        // as a run it started detached.
        if let Some(id) = outcome.moved_to_background {
            let after = outcome.moved_after.unwrap_or_default().as_secs().max(1);
            let bounded = self.bound(&outcome.stdout, &outcome.stderr);
            let mut result = json!({
                "id": id,
                "status": "running",
                "timed_out": true,
                "notice": format!(
                    "moved to background after {after} s; read it with `shell_output`"
                ),
                "stdout": bounded.stdout,
                "stderr": bounded.stderr,
                "truncated": bounded.truncated,
                "sandboxed": outcome.sandboxed,
            });
            crate::agent::tools::omit_default_flag(&mut result, "truncated", false);
            if let Some(notice) = lane_notice {
                result["lane_notice"] = Value::String(notice.to_owned());
            }
            return Ok(result);
        }
        // A successful single-file view is a read witness: the file's digest
        // right now guards a later edit that carries no digest. Anything
        // compound, redirected, piped or option-shaped records nothing.
        if outcome.exit_code == Some(0)
            && let Some(target) = &self.read_witness
        {
            super::view::record(target, command);
        }
        // The shape is the model's, so the flags it reads at their default are
        // left out: a cancelled run returned above, so `cancelled` is always
        // false here and is not carried at all, and `truncated`/`timed_out`
        // are served only when they are true. `sandboxed` stays either way:
        // it says which lane the command took, not a default.
        let bounded = self.bound(&outcome.stdout, &outcome.stderr);
        let mut result = json!({
            "exit_code": outcome.exit_code,
            "stdout": bounded.stdout,
            "stderr": bounded.stderr,
            "truncated": outcome.truncated || bounded.truncated,
            "timed_out": outcome.timed_out,
            "sandboxed": outcome.sandboxed,
        });
        for flag in ["truncated", "timed_out"] {
            crate::agent::tools::omit_default_flag(&mut result, flag, false);
        }
        // Where the lane was rerouted to the sandbox because no operator could
        // answer, the reason rides with the result: the model reads where its
        // command ran, and why, from the one place it is already reading.
        if let Some(notice) = lane_notice {
            result["lane_notice"] = Value::String(notice.to_owned());
        }
        Ok(result)
    }
}

#[async_trait]
impl ApprovalDecider for ShellTool {
    /// The lane decision is the whole of what a `shell` call is asked about.
    /// The two readers are not: they reach only runs this shell started, so
    /// they carry no authority a start did not already carry — and asking the
    /// operator to read back a command they already approved would be a
    /// prompt with no decision in it.
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        match operation(&tool.name) {
            Some(Operation::Run) => self.policy.approve(arguments).await,
            Some(Operation::Output | Operation::Kill) => self.background,
            None => false,
        }
    }

    fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> Option<bool> {
        match operation(&tool.name) {
            Some(Operation::Run) => Some(self.policy.would_ask(arguments)),
            Some(Operation::Output | Operation::Kill) => Some(false),
            None => Some(false),
        }
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        match operation(&tool.name) {
            Some(Operation::Run) => self.policy.refusal_detail(arguments),
            Some(Operation::Output | Operation::Kill) => (!self.background).then(|| {
                "this shell starts no background run, so there is none to read or kill".to_owned()
            }),
            None => None,
        }
    }
}

#[async_trait]
impl OwnApproval for ShellTool {
    /// The shell owns its whole family — the run and the two calls that reach
    /// a background run it started — through [`operation`], the same table
    /// [`ShellTool::call`] dispatches on.
    fn owns(&self, name: &str) -> bool {
        operation(name).is_some()
    }
}

#[async_trait]
impl ToolExecutor for ShellTool {
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: Value,
        _context: ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        self.call(name, &arguments).await
    }

    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        self.call(name, &arguments).await
    }

    fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        ShellTool::take_background_exits(self)
    }

    fn background_live(&self) -> bool {
        ShellTool::background_live(self)
    }

    fn background_notice_pending(&self) -> bool {
        ShellTool::background_notice_pending(self)
    }

    async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        ShellTool::wait_background_exit(self, deadline, cancellation).await
    }
}
