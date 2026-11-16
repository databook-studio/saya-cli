//! The consent seams a call with an effect outside the workspace passes
//! through, shared by every tool that has one.
//!
//! **Who is asked.** [`Operator`] is the human a call is put to: the
//! interactive panel in a live session, nobody at all in a headless run. A
//! tool asks it through a [`Prompt`], whose line names the caller — the root
//! names its job, a helper names itself and its job — in the tool's own
//! words ([`Caller::line`]). [`SerialisedOperator`] wraps the operator once
//! per job, so several callers asking at once queue: one prompt is pending at
//! a time, and a waiting ask still honours cancellation.
//!
//! **Who decides.** A tool that carries its own lane decision — the `shell`
//! (host versus sandbox, and whether the mode asks at all) and `web_fetch`
//! (the mode, and whether an operator is there to answer) — is not decided
//! by the surface's coarse effect gate, which knows only that the call
//! touches the world outside. [`OwnApproval`] is how such a tool says so, and
//! [`OwnedToolApproval`] is the router: a call a surface's own tool owns is
//! decided by that tool, and every other call keeps the wrapped policy's
//! answer unchanged.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, CancellationToken, ImmutableAuthorityRefusal, ToolDefinition};
use saya_types::{AgentId, JobId};
use serde_json::Value;

/// The operator a call is put to. `subject` names who is asking, so one panel
/// can answer for every caller without either surface wording its own prompt.
/// Deliberately not [`ApprovalDecider`]: the model-facing approval contract
/// has no place for a subject line.
#[async_trait]
pub(crate) trait Operator: Send + Sync {
    /// Puts one call to the operator under `subject`, and returns the answer.
    /// A refusal is a normal denial.
    async fn ask(&self, subject: &str, arguments: &Value) -> bool;
}

/// Who is asking, as the operator's prompt says it: the root's own tool names
/// its job, a helper's names the helper and the job it works for, and the
/// interactive session's names no job at all — it belongs to no job. The
/// wording of the line belongs to the tool being asked about, so every surface
/// that puts a call to the operator names the same caller the same way.
#[derive(Debug, Clone)]
pub(crate) enum Caller {
    /// The root's own tool: `… for job <id>`.
    Root { job_id: JobId },
    /// One helper's tool: `Helper <agent-id> wants to … for job <id>`.
    Helper { agent_id: AgentId, job_id: JobId },
    /// The interactive session's tool: no job, so no job is named.
    Session,
}

impl Caller {
    /// The line the operator's prompt opens with, in the calling tool's own
    /// words: the `imperative` a root's or a session's line is announced with
    /// (`Run a shell command`, `Fetch a web page`), and the `verb` a helper's
    /// sentence uses (`run a shell command`).
    pub(crate) fn line(&self, imperative: &str, verb: &str) -> String {
        match self {
            Caller::Root { job_id } => format!("{imperative} for job {job_id}"),
            Caller::Helper { agent_id, job_id } => {
                format!("Helper {agent_id} wants to {verb} for job {job_id}")
            }
            Caller::Session => imperative.to_owned(),
        }
    }
}

/// The operator and the line it will be shown, as one value: a call with nobody
/// to ask has no line to show, and a line with no operator would never be
/// shown. `None` on a tool means its mode has no ask — a headless `ask` job, or
/// a mode that refuses the call outright.
pub(crate) struct Prompt {
    operator: Arc<dyn Operator>,
    line: String,
}

impl Prompt {
    pub(crate) fn new(operator: Arc<dyn Operator>, line: String) -> Self {
        Self { operator, line }
    }

    /// Puts one call to the operator under this prompt's line.
    pub(crate) async fn ask(&self, arguments: &Value) -> bool {
        self.operator.ask(&self.line, arguments).await
    }
}

/// Serialises asks so one is pending at a time, whatever the caller:
/// several helpers of one job may ask at once, and the operator answers them
/// one at a time, in the order the gate admits them. A waiting ask still
/// honours cancellation — a cancelled job never holds the gate and never
/// reaches the operator.
pub(crate) struct SerialisedOperator {
    inner: Arc<dyn Operator>,
    gate: tokio::sync::Mutex<()>,
    cancellation: CancellationToken,
}

impl SerialisedOperator {
    pub(crate) fn new(inner: Arc<dyn Operator>, cancellation: CancellationToken) -> Self {
        Self {
            inner,
            gate: tokio::sync::Mutex::new(()),
            cancellation,
        }
    }
}

#[async_trait]
impl Operator for SerialisedOperator {
    async fn ask(&self, subject: &str, arguments: &Value) -> bool {
        let guard = tokio::select! {
            _ = self.cancellation.cancelled() => return false,
            guard = self.gate.lock() => guard,
        };
        let answer = self.inner.ask(subject, arguments).await;
        drop(guard);
        answer
    }
}

/// A tool that decides the calls it owns, rather than leaving them to the
/// surface's coarse effect gate. `owns` is the tool's own name table, so a
/// dispatch and an advertisement cannot disagree about which tool a word
/// names.
#[async_trait]
pub(crate) trait OwnApproval: ApprovalDecider {
    /// Whether this tool owns `name`.
    fn owns(&self, name: &str) -> bool;
}

/// Routes a call to the surface's own tool when one owns it, and to the
/// wrapped decider otherwise. A surface whose episode approval is a single
/// decider — a helper's exact-write admission — wraps it in this, so a call
/// the surface's tool owns is decided exactly as the surface that advertises
/// it decides it, while every other call keeps the wrapped policy's answer.
pub(crate) struct OwnedToolApproval {
    owners: Vec<Arc<dyn OwnApproval>>,
    inner: Arc<dyn ApprovalDecider>,
}

impl OwnedToolApproval {
    pub(crate) fn new(owners: Vec<Arc<dyn OwnApproval>>, inner: Arc<dyn ApprovalDecider>) -> Self {
        Self { owners, inner }
    }

    /// The tool that owns `tool`, where this surface has one and the name is
    /// one the tool's own table routes.
    fn owner_for(&self, tool: &ToolDefinition) -> Option<&dyn OwnApproval> {
        self.owners
            .iter()
            .find(|owner| owner.owns(&tool.name))
            .map(AsRef::as_ref)
    }
}

#[async_trait]
impl ApprovalDecider for OwnedToolApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        match self.owner_for(tool) {
            Some(owner) => owner.approve(tool, arguments).await,
            None => self.inner.approve(tool, arguments).await,
        }
    }

    fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> Option<bool> {
        match self.owner_for(tool) {
            Some(owner) => owner.would_ask(tool, arguments),
            None => self.inner.would_ask(tool, arguments),
        }
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        match self.owner_for(tool) {
            Some(owner) => owner.refusal_detail(tool, arguments),
            None => self.inner.refusal_detail(tool, arguments),
        }
    }

    /// The wrapped decider's answer, unchanged: a call an owned tool decided
    /// never carries this class (its lane decision has already refused it,
    /// and it writes nothing this decider admits), so wrapping must not
    /// swallow the class for every other tool.
    fn immutable_authority_refusal(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<ImmutableAuthorityRefusal> {
        self.inner.immutable_authority_refusal(tool, arguments)
    }
}
