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
pub(crate) mod evidence;
pub(crate) mod family;
mod family_executor;
#[cfg(test)]
mod family_tests;
#[cfg(test)]
mod learning_fixture;
#[cfg(test)]
mod learning_tests;
#[cfg(test)]
mod prompt_order_twin_tests;
#[cfg(test)]
mod prompt_twin_fixture;
#[cfg(test)]
mod prompt_twin_tests;
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
mod state;
mod surface;
mod turn_policy;

use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, ContextBlock, RunContext, RunPlugin, RunSurface, TokenUsage,
    ToolDefinition, ToolExecutor, TurnOutcome, TurnPolicy,
};

use crate::agent::knowledge_event::knowledge_supplied_event;
use crate::agent::tools::workspace::is_tool_name;
use crate::agent::tools::{CaptureHook, DatabaseTools, clarification};
use crate::contracts::RecallReceipt;

pub(crate) use family::{Bound, Family};
pub(crate) use learning::LearningSource;
pub(crate) use recall::RecallSource;
// The two surfaces' constructors and gates live in `surface`; the chat's gate
// type is composed by the chat's own composition, so it is reachable there.
pub(crate) use surface::ChatDatabaseFlags;
pub(crate) use turn_policy::DatabaseBudget;

use surface::{ChatSurface, Surface};

/// Every database tool name either surface can offer. The registry checks
/// collisions against this list once, so it is the whole of what the plugin may
/// ever route — never only what it advertises on this run. No workspace file
/// tool is here: those belong to the core.
const DECLARED_TOOLS: [&str; 11] = [
    "use_database",
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
    /// Whether this run advertises the database family yet, and the evidence
    /// that turns it on: a bound profile alone is not the request to use it.
    family: Arc<Family>,
    /// The chat surface's executor, composed once: the ask's wrapper over the
    /// surface's own tools. Composed here rather than per call, so every route
    /// to a chat database name is the same executor.
    executor: OnceLock<Arc<dyn ToolExecutor>>,
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
    /// Attaches the run's recall source: with one, `context()` recalls this
    /// turn's database contracts. Without one, it contributes nothing.
    pub(crate) fn with_recall(mut self, source: RecallSource) -> Self {
        self.recall = Some(source);
        self
    }

    /// Attaches the run's family rule: whether this run advertises the database
    /// family yet, and the evidence that turns it on. A surface that never
    /// states one keeps the family on from its first turn ([`Family::always`])
    /// — the constructor's default.
    pub(crate) fn with_family(mut self, family: Arc<Family>) -> Self {
        self.family = family;
        self
    }

    /// Attaches the run's learning source: with one, `on_turn_finish()` learns
    /// from the answered turn. Without one, it contributes nothing and reports
    /// no usage.
    pub(crate) fn with_learning(mut self, source: LearningSource) -> Self {
        self.learning = Some(source);
        self
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
        // The turn's own words are the evidence the family is advertised from,
        // and the same latch gates what the turn is told: a request that never
        // showed database work is not handed database contracts.
        if !self.family.note(prompt) {
            return Vec::new();
        }
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
        // Nothing was recalled before the family is on, so there is nothing to
        // report: a turn that never reached the database says no line at all,
        // never "memory supplied · 0 claims".
        if !self.family.latched() {
            return None;
        }
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
        // The section is the family's too: it states the answer contract, the
        // dialect and the session's databases, which a request that never
        // mentioned a database must not pay for.
        if !self.family.latched() {
            return None;
        }
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
            //
            // While the family is off, the whole of what the plugin offers is
            // the one ask: the family is appended after it once the latch
            // lands, so the list only ever grows at its end.
            Surface::Chat(ChatSurface { flags, .. }) => {
                let mut definitions: Vec<ToolDefinition> = Vec::new();
                definitions.extend(self.family.announces());
                if !self.family.latched() {
                    return definitions;
                }
                definitions.extend(
                    DatabaseTools::definitions(
                        flags.allow_query_data,
                        flags.has_state_store,
                        flags.permit_candidate_writes,
                        false,
                    )
                    .into_iter()
                    .filter(|definition| !is_tool_name(&definition.name)),
                );
                clarification::advertise(&mut definitions);
                definitions
            }
            Surface::Job(job) => job.tools.definitions(job.view.as_ref()),
        }
    }

    /// The executor that serves this plugin's tools: the chat surface's own
    /// toolset with the ask's wrapper over it, or the job surface's adapter.
    ///
    /// The chat surface's executor is composed once and reused: the ask's
    /// wrapper over the surface's own tools, so every route to a chat database
    /// name is the one executor.
    fn executor(&self) -> Option<Arc<dyn ToolExecutor>> {
        match &self.surface {
            // The chat surface's executor is `DatabaseTools` itself — never a
            // wrapper the route cannot see — so the tools the loop already runs
            // are the ones this plugin routes to, and they are the ones that
            // capture: the run's hook is installed on them here, before the
            // loop can call them. No hook installs nothing, leaving the tools
            // as built. `use_database` is served beside them rather than by
            // them: it is the plugin's own ask, not a database query.
            Surface::Chat(ChatSurface { tools, .. }) => {
                Some(Arc::clone(self.executor.get_or_init(|| {
                    tools.set_capture_hook(self.capture_hook());
                    Arc::new(family_executor::UseDatabase::new(
                        tools.clone(),
                        Arc::clone(&self.family),
                    )) as Arc<dyn ToolExecutor>
                })))
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
