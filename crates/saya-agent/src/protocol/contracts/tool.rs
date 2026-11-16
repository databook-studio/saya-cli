//! Tool contracts: definitions, effects, the executor trait, and the
//! local-state effect a tool may have.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::ToolError;
use crate::protocol::streaming::CancellationToken;

/// What local state a tool may touch — contracts, the schema cache, run
/// workspaces, anything persisted on the user's machine. Declared per tool so
/// "may this tool write local state?" is a property the loop reads rather than
/// something inferred from a tool's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LocalStateEffect {
    /// Touches no local state.
    #[default]
    None,
    /// Reads local state (contracts, cache) and writes nothing.
    Read,
    /// May persist a *candidate* claim. Never a confirmed one — confirmation is a
    /// human action and has no tool.
    WriteCandidate,
    /// May write files inside the run workspace (contained, atomic, never
    /// executable). Gated like [`LocalStateEffect::WriteCandidate`]: the loop
    /// refuses it unless the runner was constructed with workspace writes
    /// permitted, so a registered write tool cannot write merely by being
    /// registered.
    WriteWorkspace,
    /// May write session-scoped metadata only — the task list on the session
    /// record, never workspace files, never the database, never contracts or
    /// the cache. A tool reaching any of those declares the effect for
    /// *that*, not this one.
    WriteSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEffect {
    pub database_data: bool,
    pub external_side_effect: bool,
    pub requires_approval: bool,
    /// What local state this tool may touch. `#[serde(default)]` keeps the
    /// pre-3a serialized form (no key) deserializing to `None`.
    #[serde(default)]
    pub local_state: LocalStateEffect,
}

/// Whether calls to this tool may overlap other calls in one assistant message.
/// This stays separate from permissions and effects: an allowed read can still
/// contend with another operation or require ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolConcurrency {
    /// Preserve call order. Serialized definitions from an earlier build also
    /// use this default.
    #[default]
    Serial,
    /// The implementation has been audited to run independently.
    Concurrent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub read_only: bool,
    pub parameters: serde_json::Value,
    pub effect: ToolEffect,
    /// Explicit scheduling metadata. Omitting it must never grant concurrency.
    #[serde(default)]
    pub concurrency: ToolConcurrency,
    /// One-line summary the loop reports when this tool succeeds, overriding
    /// the generic read-only/write wording — a tool whose action is neither
    /// "read-only database" nor "local-state write" (e.g. one that writes a
    /// file and opens a browser) states what it actually did. The failure
    /// summary is derived from the same text and always keeps the substring
    /// "failed", which the status derivation and the statement-outcome memory
    /// key on. `#[serde(default)]` accepts serialized forms without the key;
    /// skipping `None` keeps serialized definitions byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<String>,
}

/// Per-call limits and cancellation state supplied by the agent loop.
#[derive(Clone)]
pub struct ToolExecutionContext {
    pub result_cap: usize,
    pub cancellation: CancellationToken,
}

/// One background shell run that ended since the loop last asked. The loop
/// appends one short notice for each and carries no command output: the model
/// reads the output with `shell_output`, and the notice is deliberately
/// neutral for the progress classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundExit {
    pub id: u64,
    pub exit_code: Option<i32>,
    /// `shell_kill` ended the run, rather than the command ending on its own.
    /// A run ended by the job's cancellation is the surface ending and is
    /// never noticed.
    pub killed: bool,
    pub elapsed: Duration,
    /// Every output line the run wrote, counted across all reads.
    pub output_lines: u64,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Invocation owners call this after the agent loop returns, including errors
    /// and cancellation, before reporting joined completion. Executors retaining
    /// processes must stop and join them here; uncertain cleanup returns an error.
    /// Pause boundaries call this too; implementations must be idempotent and
    /// permit new work after a successful settlement and explicit resume.
    /// Stateless executors have no retained work to settle.
    async fn settle_invocation(&self) -> Result<(), ToolError> {
        Ok(())
    }

    /// The agent loop this executor served has ended — successfully, with an
    /// error, or cancelled. A decorator that observes a turn's end (the hook
    /// gate's `Stop`) runs it here; it decides nothing, because the outcome
    /// has already been returned to the caller. The default does nothing:
    /// most executors have no lifecycle to observe.
    async fn turn_finished(&self) {}

    /// A user-turn note the loop replaces before each request. Empty means
    /// the previous note is removed. The coding root uses this to show the
    /// live `checklist` list on the turn after it changed — and nothing on
    /// the turns it did not — without rewriting the cached prefix.
    fn turn_context_note(&self) -> Option<String> {
        None
    }

    /// The definitions the loop advertises on the next request, for a surface
    /// whose tool list can change while the run is going. `None` — the
    /// default, and every surface but the coding root — keeps the list the
    /// run was constructed with. The loop re-reads this before every request,
    /// so a surface returning `Some` must return the same list on every later
    /// call once it stops growing: the coding root only ever appends, once,
    /// so the cached prefix changes once.
    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        None
    }

    /// The background shell runs that ended since the loop last called this.
    /// The loop appends one coalesced notice for them to the next request. A
    /// surface with no background shell returns nothing.
    fn take_background_exits(&self) -> Vec<BackgroundExit> {
        Vec::new()
    }

    /// Whether any background shell run is still going. The loop reads this
    /// only on a headless run, when the model's reply carried no tool calls
    /// and the run may wait for the next exit instead of finishing.
    fn background_live(&self) -> bool {
        false
    }

    /// Whether a run has ended and its notice has not been delivered. The loop
    /// reads this before it waits, so an exit that already happened is never
    /// waited on.
    fn background_notice_pending(&self) -> bool {
        false
    }

    /// Waits until a background run ends, the deadline passes, or the run is
    /// cancelled. Returns whether a notice is ready to deliver. The default —
    /// every surface with no background shell — waits for nothing.
    async fn wait_background_exit(
        &self,
        _deadline: Instant,
        _cancellation: &CancellationToken,
    ) -> bool {
        false
    }

    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError>;

    /// Executes with the exact model-result cap assigned to this call. Existing
    /// executors need no context and keep `execute`; executors that publish a
    /// model-view side channel can use this to apply the same cut.
    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        _result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        self.execute(name, arguments).await
    }

    /// Executes with the exact loop-assigned result cap and cancellation token.
    /// Existing cap-aware executors stay compatible through the default.
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.execute_with_result_cap(name, arguments, context.result_cap)
            .await
    }
}
