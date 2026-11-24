//! The question a job's write-shaped call is put to, and the mode that decides
//! whether it is asked at all.
//!
//! The exact-write admission decides *what* a call may touch; this decides
//! whether the job's current consent covers it. The capability runs first and
//! still refuses whatever it refuses today. Only a call it admits is then
//! resolved by the job's mode — the same cell the job's `shell` reads, so a
//! mode stated during the run decides the next call:
//!
//! - `bypass` and `accept-edits`: admitted without asking, which is what a
//!   detached composition has always done;
//! - `read-only` and `never`: refused, and nobody is asked — no answer could
//!   widen the mode;
//! - `ask`: admitted when the job's grant set already holds
//!   `workspace-write`, and otherwise put to the job's operator, under a
//!   subject line that names the caller and arguments that name the path.
//!
//! A write the surface does not offer at all — a closed plan posture, or a
//! `[tools] allow` list that dropped it — is refused before any question: the
//! executor applies the same gate, so the question could only end in a refusal.
//!
//! A "yes for this run" records `workspace-write` in the operator's own store,
//! so the next write of the same job is not asked again; a refusal, a dead
//! prompt and a cancelled job all refuse.
//!
//! A job with no operator attached composes no [`WriteAsk`] at all: a detached
//! run has nobody to ask, so it keeps the consent it composed — an
//! edit-capable mode, or one that refuses the call outright — exactly as
//! before.

use std::sync::Arc;

use saya_agent::{
    ApprovalDecider, ApprovalPolicy, SessionGrants, ToolDefinition, WORKSPACE_WRITE_TOKEN,
};
use saya_types::AgentId;
use serde_json::Value;

use super::consent::{Answer, Caller, Operator, Prompt};
use super::shell_policy::ModeCell;
use super::workspace::is_write_tool;
use crate::commands::coding::NativeCodingJob;

/// The subject line a write-shaped call is put under: the imperative a root's
/// line opens with, and the verb a helper's sentence uses.
const IMPERATIVE: &str = "Change a workspace file";
const VERB: &str = "change a workspace file";

/// The line the operator reads for `caller` — who is asking, in this family's
/// own words. Read by the tests that assert the wording, so a test and the
/// prompt cannot state it differently.
pub(crate) fn line(caller: &Caller) -> String {
    caller.line(IMPERATIVE, VERB)
}

/// The root's write ask, when an operator is attached to `job`: the shape a
/// job started from a surface with a person in front of it runs in. A detached
/// run has nobody to ask, so it composes none and keeps the consent it stated.
pub(crate) fn for_root(job: &NativeCodingJob) -> Option<WriteAsk> {
    let operator = job.operator()?;
    let caller = Caller::Root {
        job_id: job.job_id().clone(),
    };
    Some(
        WriteAsk::new(job.approval_mode(), operator, &caller)
            .with_launch_grants(job.launch_grants()),
    )
}

/// One helper's write ask, under a line that names the helper: the same ask,
/// the same mode cell, the same grant store. `None` where the job has nobody
/// to ask, so a detached composition keeps the capability's answer.
pub(crate) fn for_helper(job: &NativeCodingJob, agent_id: &AgentId) -> Option<Arc<WriteAsk>> {
    let operator = job.operator()?;
    let caller = Caller::Helper {
        agent_id: agent_id.clone(),
        job_id: job.job_id().clone(),
    };
    Some(Arc::new(
        WriteAsk::new(job.approval_mode(), operator, &caller)
            .with_launch_grants(job.launch_grants()),
    ))
}

/// A write this surface does not offer — a closed plan gate, or a `[tools]
/// allow` list that dropped it — is refused before anyone is asked: the
/// executor applies the same gate, so a question about it could only end in a
/// refusal.
pub(crate) fn hidden_write(definition: &ToolDefinition, advertised: impl Fn(&str) -> bool) -> bool {
    is_write_tool(&definition.name) && !advertised(&definition.name)
}

/// One job's mode and operator: what a write-shaped call the capability admits
/// is resolved by.
pub(crate) struct WriteAsk {
    /// The job's own mode cell, read at every call.
    mode: ModeCell,
    /// The operator and the subject line it is shown, plus the store a session
    /// answer is recorded into.
    prompt: Prompt,
    /// The launch's own seed store (`--allow`), when the surface stated one: a
    /// token it holds pre-answers the family its mode would have asked about,
    /// exactly as the tool advertisement and the exact-write admission read
    /// that same store.
    launch: Option<SessionGrants>,
}

impl WriteAsk {
    /// The ask of one caller of one job, built where the job states its mode
    /// and its operator.
    pub(crate) fn new(mode: ModeCell, operator: Arc<dyn Operator>, caller: &Caller) -> Self {
        let grants = operator.grants();
        Self {
            mode,
            prompt: Prompt::new(operator, line(caller)).with_grants(grants),
            launch: None,
        }
    }

    /// States the launch's own seed store (`--allow`): a family it already
    /// grants is pre-answered here too, so a headless run is admitted exactly
    /// what the launch stated rather than asked for it.
    pub(crate) fn with_launch_grants(mut self, grants: SessionGrants) -> Self {
        self.launch = Some(grants);
        self
    }

    /// Whether this caller's family is already granted — by a session answer
    /// the operator recorded, or by the launch's own seeds.
    fn held(&self) -> bool {
        self.prompt.granted(WORKSPACE_WRITE_TOKEN)
            || self
                .launch
                .as_ref()
                .is_some_and(|grants| grants.is_granted(WORKSPACE_WRITE_TOKEN))
    }

    /// Whether this ask would put the call to the operator, as the mode and
    /// the stores stand now.
    pub(crate) fn asks(&self) -> bool {
        self.mode.get() == ApprovalPolicy::Ask && !self.held()
    }

    /// One call's admission: the capability first, because a call it refuses is
    /// refused whatever the mode says, and then the mode's answer for a
    /// write-shaped call.
    pub(crate) async fn approve(
        &self,
        capability: &dyn ApprovalDecider,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> bool {
        if !capability.approve(tool, arguments).await {
            return false;
        }
        if !is_write_tool(&tool.name) {
            return true;
        }
        self.resolve(arguments).await
    }

    /// The mode's answer for a write-shaped call the capability has already
    /// admitted.
    pub(crate) async fn resolve(&self, arguments: &Value) -> bool {
        match self.mode.get() {
            ApprovalPolicy::Bypass | ApprovalPolicy::AcceptEdits => true,
            ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => false,
            ApprovalPolicy::Ask => self.asked(arguments).await,
        }
    }

    /// The `ask` arm: a grant the job already holds pre-answers the call, and
    /// only a yes that names the run is recorded.
    async fn asked(&self, arguments: &Value) -> bool {
        if self.held() {
            return true;
        }
        match self.prompt.answer(arguments).await {
            Answer::Deny => false,
            Answer::Once => true,
            Answer::Session => {
                self.prompt.record(WORKSPACE_WRITE_TOKEN);
                true
            }
        }
    }

    /// Whether this call would reach the operator, for a caller that decides
    /// what to do with an ask before making it. Every call that is not
    /// write-shaped keeps the capability's own answer.
    pub(crate) fn would_ask(
        &self,
        capability: &dyn ApprovalDecider,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<bool> {
        if !is_write_tool(&tool.name) {
            return capability.would_ask(tool, arguments);
        }
        Some(self.asks())
    }
}
