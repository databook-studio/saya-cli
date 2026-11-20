//! The database plugin: the database tools the chat and the coding job offer
//! today, behind the [`RunPlugin`] contract.
//!
//! Both surfaces already build the same toolset — `DatabaseTools` — and differ
//! only in how they narrow it: the chat advertises the whole non-workspace
//! toolset, while a coding job advertises the standard read-only pair over the
//! profiles its consent selected. The plugin keeps that difference in its two
//! constructors so the core sees one plugin with one name.

#[cfg(test)]
mod activation_tests;
#[cfg(test)]
mod learning_fixture;
#[cfg(test)]
mod learning_tests;
#[cfg(test)]
mod recall_fixture;
#[cfg(test)]
mod recall_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod turn_policy_tests;

mod job_adapter;
mod learning;
mod recall;
mod turn_policy;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{
    AgentEventSink, ContextBlock, RunContext, RunPlugin, RunSurface, TokenUsage, ToolDefinition,
    ToolExecutor, TurnOutcome, TurnPolicy,
};
use saya_harness::coding::policy::ChildAuthorityView;

use crate::agent::tools::DatabaseTools;
use crate::agent::tools::workspace::is_tool_name;
use crate::commands::coding::database::CodingDatabaseTools;
use crate::contracts::RecallReceipt;

pub(crate) use learning::LearningSource;
pub(crate) use recall::RecallSource;
pub(crate) use turn_policy::SessionBudget;

/// Every database tool name either surface can offer. The registry checks
/// collisions against this list once, so it is the whole of what the plugin may
/// ever route — never only what it advertises on this run. No workspace file
/// tool is here: those belong to the core.
const DECLARED_TOOLS: [&str; 10] = [
    "schema_discovery",
    "request_clarification",
    "bounded_sql_query",
    "bounded_sql_query_all",
    "result_shape",
    "column_health",
    "join_check",
    "designate_answer",
    "contract_search",
    "contract_read",
];

/// The gates the chat surface passes to `DatabaseTools::definitions`. The
/// workspace-write flag is not here: the workspace tools are the core's, so
/// this plugin always asks for their absent form.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChatDatabaseFlags {
    pub(crate) allow_query_data: bool,
    pub(crate) has_state_store: bool,
    pub(crate) permit_candidate_writes: bool,
}

/// The chat surface's toolset, the gates it advertises under, and the
/// `[limits.session]` budget a chat turn of this run is bounded by.
struct ChatSurface {
    tools: Arc<DatabaseTools>,
    flags: ChatDatabaseFlags,
    session: SessionBudget,
}

/// A coding job's toolset and the authority view it serves.
struct JobSurface {
    tools: Arc<CodingDatabaseTools>,
    view: Option<ChildAuthorityView>,
}

/// Which surface's tools this plugin was built for, and the narrowed toolset it
/// serves them from. The job side is boxed: a `ChildAuthorityView` is far larger
/// than the chat side's flags, and the plugin is not a hot path that wants the
/// larger shape inline.
enum Surface {
    Chat(ChatSurface),
    Job(Box<JobSurface>),
}

pub(crate) struct DatabasePlugin {
    surface: Surface,
    /// What `context()` recalls from, when the run configured memory.
    recall: Option<RecallSource>,
    /// What `on_turn_finish()` learns from, when the run configured memory.
    learning: Option<LearningSource>,
    /// The last recall's receipt, for the executor that reports what reached
    /// the model. A mutex because `context()` takes `&self` and the executor
    /// reads it after the hook ran.
    last_receipt: Mutex<Option<Arc<RecallReceipt>>>,
}

impl DatabasePlugin {
    /// The chat surface's database tools, over the session budget its turns
    /// are bounded by.
    pub(crate) fn for_chat(
        tools: Arc<DatabaseTools>,
        flags: ChatDatabaseFlags,
        session: SessionBudget,
    ) -> Self {
        Self {
            surface: Surface::Chat(ChatSurface {
                tools,
                flags,
                session,
            }),
            recall: None,
            learning: None,
            last_receipt: Mutex::new(None),
        }
    }

    /// A coding job's database tools, narrowed to the profiles `view` admits.
    pub(crate) fn for_job(
        tools: Arc<CodingDatabaseTools>,
        view: Option<ChildAuthorityView>,
    ) -> Self {
        Self {
            surface: Surface::Job(Box::new(JobSurface { tools, view })),
            recall: None,
            learning: None,
            last_receipt: Mutex::new(None),
        }
    }

    /// Attaches the run's recall source: with one, `context()` recalls this
    /// turn's database contracts. Without one, it contributes nothing.
    pub(crate) fn with_recall(mut self, source: RecallSource) -> Self {
        self.recall = Some(source);
        self
    }

    /// Attaches the run's learning source: with one, `on_turn_finish()` learns
    /// from the answered turn. Without one, it contributes nothing and reports
    /// no usage.
    pub(crate) fn with_learning(mut self, source: LearningSource) -> Self {
        self.learning = Some(source);
        self
    }

    /// The receipt from the last `context()` call, or `None` when the hook
    /// never recalled — no source attached, or not called yet.
    pub(crate) fn last_receipt(&self) -> Option<Arc<RecallReceipt>> {
        match self.last_receipt.lock() {
            Ok(slot) => slot.clone(),
            // A poisoned lock still holds what the hook wrote: the receipt is a
            // plain value, so read it rather than lose it to an unrelated panic.
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Keeps `receipt` for the executor. One receipt per turn: the newest
    /// recall wins, exactly as the chat's turn reports its own.
    fn store_receipt(&self, receipt: RecallReceipt) {
        let mut slot = match self.last_receipt.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some(Arc::new(receipt));
    }
}

#[async_trait]
impl RunPlugin for DatabasePlugin {
    fn name(&self) -> &'static str {
        "database"
    }

    fn declared_tools(&self) -> &'static [&'static str] {
        &DECLARED_TOOLS
    }

    /// Whether this run has a database side at all.
    ///
    /// The rule is the surface's: a chat run activates the plugin only when it
    /// was resolved with at least one database profile — a chat with none has
    /// nothing to read and nothing to ask — while a headless, background or
    /// helper run always does, because the job that started it already decided
    /// its database scope before the run existed. The chat is not wired through
    /// the registry yet; this is the rule it will activate under.
    fn activate(&self, ctx: &RunContext) -> bool {
        match ctx.surface {
            RunSurface::Chat => ctx.database_profiles > 0,
            RunSurface::Headless | RunSurface::Background | RunSurface::Helper => true,
        }
    }

    /// The run's database contracts for this turn's prompt, recalled through the
    /// one function the chat's turn calls — and the receipt of what reached the
    /// model, kept for the executor that reports it. No source attached means
    /// the run configured no memory: nothing to contribute, no receipt.
    async fn context(&self, _ctx: &RunContext, prompt: &str) -> Vec<ContextBlock> {
        let Some(source) = self.recall.as_ref() else {
            return Vec::new();
        };
        let (blocks, receipt) = source.recall(prompt).await;
        self.store_receipt(receipt);
        blocks
    }

    fn definitions(&self, _ctx: &RunContext) -> Vec<ToolDefinition> {
        match &self.surface {
            // The workspace file tools ride the same list the chat builds today,
            // and are the core's: the plugin offers the rest, in the order the
            // chat has always advertised it, so the cached prefix is unchanged.
            Surface::Chat(ChatSurface { flags, .. }) => DatabaseTools::definitions(
                flags.allow_query_data,
                flags.has_state_store,
                flags.permit_candidate_writes,
                false,
            )
            .into_iter()
            .filter(|definition| !is_tool_name(&definition.name))
            .collect(),
            Surface::Job(job) => job.tools.definitions(job.view.as_ref()),
        }
    }

    fn executor(&self) -> Option<Arc<dyn ToolExecutor>> {
        match &self.surface {
            // The chat surface's executor is `DatabaseTools` itself — never a
            // wrapper — so the tools the loop already runs are the ones this
            // plugin routes to.
            Surface::Chat(ChatSurface { tools, .. }) => Some(tools.clone()),
            Surface::Job(job) => Some(Arc::new(job_adapter::JobToolExecutor::new(
                job.tools.clone(),
                job.view.clone(),
            ))),
        }
    }

    /// The chat's question budget as the core enforces it: every turn of a chat
    /// with a database configured is a database question, and its ceilings come
    /// from `[limits.session]`. A job surface asks for none.
    fn turn_policy(&self, ctx: &RunContext) -> Option<TurnPolicy> {
        self.turn_policy_for(ctx)
    }

    /// Learns from the turn through the one function the chat's turn calls, and
    /// reports what that call spent so the run can fold it into the learning
    /// total. No source attached means the run configured no memory: nothing is
    /// learned and nothing is announced. The receipt is the one `context()`
    /// kept for this turn; when recall never ran — no recall source, or the
    /// hook not called — it is the empty receipt a recall configured off
    /// produces, so extraction sees no supplied claims rather than inventing
    /// some.
    async fn on_turn_finish(
        &self,
        _ctx: &RunContext,
        outcome: &TurnOutcome<'_>,
        sink: &dyn AgentEventSink,
    ) -> Option<TokenUsage> {
        let source = self.learning.as_ref()?;
        let receipt = self
            .last_receipt()
            .unwrap_or_else(|| Arc::new(RecallReceipt::configured_off()));
        source.learn(outcome, &receipt, sink).await
    }
}
