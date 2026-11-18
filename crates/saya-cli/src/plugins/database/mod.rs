//! The database plugin: the database tools the chat and the coding job offer
//! today, behind the [`RunPlugin`] contract.
//!
//! Both surfaces already build the same toolset — `DatabaseTools` — and differ
//! only in how they narrow it: the chat advertises the whole non-workspace
//! toolset, while a coding job advertises the standard read-only pair over the
//! profiles its consent selected. The plugin keeps that difference in its two
//! constructors so the core sees one plugin with one name.

#[cfg(test)]
mod recall_fixture;
#[cfg(test)]
mod recall_tests;
#[cfg(test)]
mod tests;

mod job_adapter;
mod recall;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{ContextBlock, RunContext, RunPlugin, ToolDefinition, ToolExecutor};
use saya_harness::coding::policy::ChildAuthorityView;

use crate::agent::tools::DatabaseTools;
use crate::agent::tools::workspace::is_tool_name;
use crate::commands::coding::database::CodingDatabaseTools;
use crate::contracts::RecallReceipt;

pub(crate) use recall::RecallSource;

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

/// The chat surface's toolset and the gates it advertises under.
struct ChatSurface {
    tools: Arc<DatabaseTools>,
    flags: ChatDatabaseFlags,
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
    /// The last recall's receipt, for the executor that reports what reached
    /// the model. A mutex because `context()` takes `&self` and the executor
    /// reads it after the hook ran.
    last_receipt: Mutex<Option<Arc<RecallReceipt>>>,
}

impl DatabasePlugin {
    /// The chat surface's database tools.
    pub(crate) fn for_chat(tools: Arc<DatabaseTools>, flags: ChatDatabaseFlags) -> Self {
        Self {
            surface: Surface::Chat(ChatSurface { tools, flags }),
            recall: None,
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
            last_receipt: Mutex::new(None),
        }
    }

    /// Attaches the run's recall source: with one, `context()` recalls this
    /// turn's database contracts. Without one, it contributes nothing.
    pub(crate) fn with_recall(mut self, source: RecallSource) -> Self {
        self.recall = Some(source);
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

    /// Every surface activates the plugin for now; the rule that keys it on the
    /// run's database profiles comes later.
    fn activate(&self, _ctx: &RunContext) -> bool {
        true
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
}
