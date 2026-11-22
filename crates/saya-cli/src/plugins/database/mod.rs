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
pub(crate) mod approval;
#[cfg(test)]
mod approval_order_tests;
#[cfg(test)]
mod approval_tests;
#[cfg(test)]
mod capture_fixture;
#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod learning_fixture;
#[cfg(test)]
mod learning_tests;
#[cfg(test)]
mod recall_fixture;
#[cfg(test)]
mod recall_tests;
#[cfg(test)]
mod section_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod turn_policy_tests;

mod job_adapter;
mod learning;
pub(crate) mod prompt;
mod recall;
mod surface;
mod turn_policy;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, ContextBlock, RunContext, RunPlugin, RunSurface, TokenUsage,
    ToolDefinition, ToolExecutor, TurnOutcome, TurnPolicy,
};

use crate::agent::knowledge_event::knowledge_supplied_event;
use crate::agent::tools::workspace::is_tool_name;
use crate::agent::tools::{CaptureEvent, CaptureHook, DatabaseTools, clarification};
use crate::contracts::RecallReceipt;

pub(crate) use learning::LearningSource;
pub(crate) use recall::RecallSource;
// The two surfaces' constructors and gates live in `surface`; the chat's gate
// type is composed by the chat's own composition, so it is reachable there.
pub(crate) use surface::ChatDatabaseFlags;
pub(crate) use turn_policy::DatabaseBudget;

use surface::{ChatSurface, Surface};

/// Writes one SQL statement into the shared slot both the plugin and the chat
/// surface's capture hook hold. A poisoned lock still holds the slot — a plain
/// string is no half-written value — so it is written rather than lost.
fn record(slot: &Mutex<Option<String>>, sql: Option<&str>) {
    let mut slot = match slot.lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    *slot = sql.map(str::to_owned);
}

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

/// Whether `name` is a name the plugin routes at all: its whole declared
/// table, never only what it advertises on this run. The lane and the registry
/// read one list, so neither can route a name the other never took.
pub(crate) fn routes(name: &str) -> bool {
    DECLARED_TOOLS.contains(&name)
}

pub(crate) struct DatabasePlugin {
    surface: Surface,
    /// The run's query-capture hook, when the surface publishes captures: the
    /// chat previews and exports the queries it runs, so its tools receive
    /// this hook. A job surface's queries are the job's own evidence, never a
    /// preview, so that arm never installs it. `None` — every caller that
    /// passes no hook — installs nothing and captures nothing.
    capture: Option<CaptureHook>,
    /// What `context()` recalls from, when the run configured memory.
    recall: Option<RecallSource>,
    /// What `on_turn_finish()` learns from, when the run configured memory.
    learning: Option<LearningSource>,
    /// The last recall's receipt, for the executor that reports what reached
    /// the model. A mutex because `context()` takes `&self` and the executor
    /// reads it after the hook ran.
    last_receipt: Mutex<Option<Arc<RecallReceipt>>>,
    /// The most recent SQL this run executed, as the run last stated it. A
    /// mutex for the same reason as the receipt: `context()` reads it through
    /// `&self` when the turn asks for its context blocks. Shared with the
    /// capture hook the chat surface's tools carry, because that hook is where
    /// a query's success is known (`record_last_sql`'s one live caller).
    last_sql: Arc<Mutex<Option<String>>>,
}

impl DatabasePlugin {
    /// Records the SQL the run just executed, so `context()` can carry the
    /// adapt-this-query hint the chat's turn carried. `None` clears it — a turn
    /// that ran no SQL states nothing about one.
    pub(crate) fn record_last_sql(&self, sql: Option<&str>) {
        record(&self.last_sql, sql);
    }

    /// The capture hook the chat surface's tools carry: the front's own hook
    /// (when it drives one) behind the record the next turn's context reads.
    ///
    /// A capture is emitted only for a query that executed — the single
    /// connection arm, on success, either as evidence or refused for its size —
    /// so this is where the run learns the SQL it most recently ran, and the
    /// hint for it rides the NEXT turn's user tail
    /// ([`Self::context`] → `last_sql_hint_block`).
    fn capture_hook(&self) -> CaptureHook {
        let last_sql = Arc::clone(&self.last_sql);
        let front = self.capture.clone();
        Arc::new(move |event: CaptureEvent| {
            let sql = match &event {
                CaptureEvent::Captured(capture) => Some(capture.sql.as_str()),
                CaptureEvent::Refused { sql, .. } => Some(sql.as_str()),
            };
            record(&last_sql, sql);
            if let Some(front) = &front {
                front(event);
            }
        })
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
    fn store_receipt(&self, receipt: Arc<RecallReceipt>) {
        let mut slot = match self.last_receipt.lock() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some(receipt);
    }

    /// The last-SQL hint block for this turn, or `None` when the run stated no
    /// SQL — the same block the chat's turn appended, built by the same
    /// function, so the two cannot word it differently.
    fn last_sql_block(&self) -> Option<ContextBlock> {
        let sql = match self.last_sql.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        sql.as_deref()
            .and_then(crate::agent::turn_context::last_sql_hint_block)
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
    ///
    /// The most recent SQL this run executed rides here too, as the same block
    /// the chat's turn appended: the hint is per turn, so it never enters the
    /// run's system prompt where it would move the prefix a provider caches.
    async fn context(&self, _ctx: &RunContext, prompt: &str) -> Vec<ContextBlock> {
        let mut blocks = Vec::new();
        if let Some(source) = self.recall.as_ref() {
            let (recalled, receipt) = source.recall(prompt).await;
            let receipt = Arc::new(receipt);
            // The tools' override detector reads the same receipt recall just
            // assembled: the chat session shares this toolset through an `Arc`,
            // so the turn's receipt is installed on it here, before any call
            // the model makes can be judged against it.
            if let Surface::Chat(chat) = &self.surface {
                chat.tools.set_recall_receipt(Some(Arc::clone(&receipt)));
            }
            self.store_receipt(receipt);
            blocks.extend(recalled);
        }
        if let Some(hint) = self.last_sql_block() {
            blocks.push(hint);
        }
        blocks
    }

    /// What this turn's recall supplied, as the one event a surface shows —
    /// stated on the chat surface from the receipt `context()` just kept.
    ///
    /// The receipt is the single source of truth: a run that configured no
    /// memory says `Off` (it did not look), a closed privacy gate says
    /// `Skipped` (it was not allowed to), and a recall that ran — even one that
    /// found nothing, or one whose store could not be read — says `Ran`. A chat
    /// with no database never reaches here (the plugin does not activate), and
    /// a job surface states none: its event stream is the job's own record.
    fn knowledge_event(&self, _ctx: &RunContext) -> Option<AgentEvent> {
        match &self.surface {
            Surface::Chat(_) => Some(knowledge_supplied_event(
                &self
                    .last_receipt()
                    .unwrap_or_else(|| Arc::new(RecallReceipt::configured_off())),
            )),
            Surface::Job(_) => None,
        }
    }

    /// The plugin's own section of the run's system prompt: the database part of
    /// it, stated on the chat surface.
    ///
    /// A chat run states the bytes `plugins::database::prompt` builds — the same
    /// text the old chat prompt renders, from the same constants. A coding job
    /// or a helper keeps its own prompt for now and states nothing here, so its
    /// request bytes do not move.
    fn system_section(&self, _ctx: &RunContext) -> Option<String> {
        match &self.surface {
            Surface::Chat(chat) => Some(chat.prompt.section()),
            Surface::Job(_) => None,
        }
    }

    fn definitions(&self, _ctx: &RunContext) -> Vec<ToolDefinition> {
        match &self.surface {
            // The workspace file tools ride the same list the chat builds today,
            // and are the core's: the plugin offers the rest, in the order the
            // chat has always advertised it, so the cached prefix is unchanged.
            // The ask is the plugin's own name, so it is placed here rather than
            // by the core list this surface builds from.
            Surface::Chat(ChatSurface { flags, .. }) => {
                let mut definitions: Vec<ToolDefinition> = DatabaseTools::definitions(
                    flags.allow_query_data,
                    flags.has_state_store,
                    flags.permit_candidate_writes,
                    false,
                )
                .into_iter()
                .filter(|definition| !is_tool_name(&definition.name))
                .collect();
                clarification::advertise(&mut definitions);
                definitions
            }
            Surface::Job(job) => job.tools.definitions(job.view.as_ref()),
        }
    }

    fn executor(&self) -> Option<Arc<dyn ToolExecutor>> {
        match &self.surface {
            // The chat surface's executor is `DatabaseTools` itself — never a
            // wrapper — so the tools the loop already runs are the ones this
            // plugin routes to, and they are the ones that capture: the run's
            // hook is installed on them here, before the loop can call them.
            // No hook installs nothing, leaving the tools as built.
            Surface::Chat(ChatSurface { tools, .. }) => {
                tools.set_capture_hook(self.capture_hook());
                Some(tools.clone())
            }
            Surface::Job(job) => Some(Arc::new(job_adapter::JobToolExecutor::new(
                job.tools.clone(),
                job.view.clone(),
            ))),
        }
    }

    /// The database budget a turn spends once it uses the database, and nothing
    /// before that: it arms on the turn's first database use, so a turn of this
    /// chat that never touches one is not bounded by it. A job surface asks for
    /// none, and no surface states an every-turn policy any more.
    fn policy_on_use(&self, ctx: &RunContext) -> Option<TurnPolicy> {
        self.policy_on_use_for(ctx)
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
