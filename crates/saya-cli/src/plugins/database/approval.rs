//! The database lane: the approval table a chat run's database tools follow.
//!
//! The chat's decider already asks about a database call this way; the job
//! root's decider never did — it admits the read pair where a profile was
//! selected and refuses the contract tools. A run that puts this plugin on the
//! chat surface gets this lane instead, and decides one call in the order every
//! other lane of the session uses: the run's narrowing, then the mode, then the
//! remembered grants, then the operator.
//!
//! | tool | ask / accept-edits | bypass | read-only | never |
//! | --- | --- | --- | --- | --- |
//! | `bounded_sql_query`, `result_shape`, `column_health`, `join_check` | ask, `sql:<connection>` pre-answers after a session answer | allow | allow | refuse |
//! | `bounded_sql_query_all` | ask every call — no token | allow | allow | refuse |
//! | `schema_discovery`, `designate_answer`, `contract_search`, `contract_read`, `request_clarification` | allow in every mode: never asked, so never consulted | | | |
//!
//! Two things are deliberately not this lane's. The data-sharing gate
//! (`allow_query_data`) refuses a row-returning call at dispatch in every mode,
//! and the SQL safety layer decides whether a statement is read-only at all;
//! the lane decides only who answers. And the plan posture narrows writes:
//! every tool here is read-shaped, so it resolves as the consent posture alone
//! would.

use saya_agent::{ApprovalPolicy, RunContext, RunSurface, SessionGrants, ToolDefinition};
use saya_config::ResolvedTools;
use serde_json::Value;

use crate::agent::tools::admits;
use crate::agent::tools::consent::{Answer, Caller, Prompt};
use crate::agent::tools::shell_policy::ModeCell;
use crate::commands::coding::NativeCodingJob;
use crate::grant_token::{SQL_FAMILY, sql_token};

/// The fan-out: one call against every connected database. It takes no grant
/// token, because its referent can grow after an approval — a database
/// connected mid-run would sit inside a grant made before it existed.
const FAN_OUT: &str = "bounded_sql_query_all";

/// The subject line a database call is put under: the imperative a root's line
/// opens with, and the verb a helper's sentence uses.
const IMPERATIVE: &str = "Run a read-only database query";
const VERB: &str = "run a read-only database query";

/// The line the operator reads for `caller` — who is asking, in this family's
/// own words. Read by the tests that assert the wording, so a test and the
/// prompt cannot state it differently.
pub(crate) fn line(caller: &Caller) -> String {
    caller.line(IMPERATIVE, VERB)
}

/// Whether a run's root takes this lane: the plugin on the chat surface with at
/// least one profile, which is the plugin's own activation rule
/// ([`saya_agent::RunPlugin::activate`]). Every job surface — `-p`, a
/// background run, a helper — keeps the decider it has.
pub(crate) fn runs_on_chat(context: &RunContext) -> bool {
    context.surface == RunSurface::Chat && context.database_profiles > 0
}

/// What one database call earns in the lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Run without asking.
    Allow,
    /// Put to the operator, with the grant token the answer would record.
    /// `None` where the call has no token — the fan-out — so an answer that
    /// names the run records nothing and the next call asks again.
    Ask { token: Option<String> },
    /// Refuse without asking: the run's narrowing, or a mode that holds none.
    Refuse,
}

/// One run's database lane: the table above, over the run's own mode cell, its
/// grant store and its operator.
pub(crate) struct DbApprovalLane {
    /// The run's mode cell, read at every call, so a mode stated between two
    /// calls decides the second.
    mode: ModeCell,
    /// The run's "yes for this run" store, consulted after the mode and before
    /// the operator: a granted token pre-answers the call the mode would have
    /// asked about.
    grants: SessionGrants,
    /// The turn's primary connection, named by the SQL family's token when the
    /// call names none. Unbound suggests no token — fail closed, never a
    /// guessed name — so such a call keeps asking.
    primary: Option<String>,
    /// The run's `[tools] allow` list: the narrowing that refuses a plugin name
    /// this surface no longer offers, in every mode.
    tools: ResolvedTools,
    /// The operator and the line it is shown, when a person is there to ask.
    /// `None` is a run nobody can answer, where an ask refuses.
    prompt: Option<Prompt>,
}

impl DbApprovalLane {
    pub(crate) fn new(
        mode: ModeCell,
        grants: SessionGrants,
        primary: Option<String>,
        tools: ResolvedTools,
        prompt: Option<Prompt>,
    ) -> Self {
        Self {
            mode,
            grants,
            primary,
            tools,
            prompt,
        }
    }

    /// The chat run's root lane for `job`: the job's own mode cell, its
    /// operator and that operator's "yes for this run" store, the job's
    /// `[tools]` list, and the primary connection the surface resolved. A call
    /// that names no connection is approved as the primary one — its token is
    /// `sql:<primary>`, the name the chat's own grants always used — while a
    /// surface that resolved no primary (`None`) suggests no token for it and
    /// such a call keeps asking.
    pub(crate) fn for_root(job: &NativeCodingJob, primary: Option<String>) -> Self {
        let operator = job.operator();
        let grants = operator
            .as_ref()
            .and_then(|operator| operator.grants())
            .unwrap_or_default();
        let caller = Caller::Root {
            job_id: job.job_id().clone(),
        };
        let prompt = operator.map(|operator| Prompt::new(operator, line(&caller)));
        Self::new(
            job.approval_mode(),
            grants,
            primary,
            job.tools().clone(),
            prompt,
        )
    }

    /// The lane a run composes, or `None` where it keeps the decider it has.
    /// This is the composition's one entry point, so the surface rule
    /// ([`runs_on_chat`]) cannot be restated somewhere else and drift: a chat
    /// run with a database takes the lane, and every job surface — `-p`, a
    /// background run, a helper — composes none.
    pub(crate) fn for_run(
        job: &NativeCodingJob,
        context: &RunContext,
        primary: Option<String>,
    ) -> Option<Self> {
        runs_on_chat(context).then(|| Self::for_root(job, primary))
    }

    /// The token an answer to this call would record: `sql:<connection>` for
    /// the SQL family, and none for the fan-out, which therefore asks every
    /// call.
    fn token_for(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        if !SQL_FAMILY.contains(&tool.name.as_str()) {
            return None;
        }
        sql_token(arguments, self.primary.as_deref())
    }

    /// The table, decided in the session's order: the run's narrowing, the
    /// mode, the remembered grant, then — in [`Self::approve`] — the operator.
    pub(crate) fn decide(&self, tool: &ToolDefinition, arguments: &Value) -> Verdict {
        // The narrowing comes first: a name this surface narrowed away is
        // refused whatever the mode says, `bypass` included, because the
        // executor would refuse it too and a mode cannot offer what the run
        // took away.
        if !admits(&self.tools, &tool.name) {
            return Verdict::Refuse;
        }
        // The never-asked read tools are not decided here in any mode — they
        // run in every one, `never` included — and neither is a name this
        // plugin does not route at all.
        if !SQL_FAMILY.contains(&tool.name.as_str()) && tool.name != FAN_OUT {
            return Verdict::Allow;
        }
        match self.mode.get() {
            ApprovalPolicy::Bypass => Verdict::Allow,
            // A read-shaped call runs unasked under `read-only`: the mode holds
            // reads and refuses the rest, and that is the engine's own
            // predicate, never this table's word for it.
            ApprovalPolicy::ReadOnly if saya_agent::read_only_permits(&tool.effect) => {
                Verdict::Allow
            }
            ApprovalPolicy::ReadOnly => Verdict::Refuse,
            ApprovalPolicy::Never => Verdict::Refuse,
            // `accept-edits` pre-answers nothing here: its one pre-answer is
            // the workspace file family, so SQL still asks under it.
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => {
                let token = self.token_for(tool, arguments);
                match &token {
                    Some(token) if self.grants.is_granted(token) => Verdict::Allow,
                    _ => Verdict::Ask { token },
                }
            }
        }
    }

    /// Decides one call, asking the run's operator where the table says so. A
    /// session answer records the call's token in the run's store, so the next
    /// call under it runs unasked; allow-once and a refusal record nothing.
    pub(crate) async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        match self.decide(tool, arguments) {
            Verdict::Allow => true,
            Verdict::Refuse => false,
            Verdict::Ask { token } => match &self.prompt {
                // Nobody to ask is a refusal: an unanswerable ask must never
                // read as consent.
                None => false,
                Some(prompt) => match prompt.answer(arguments).await {
                    Answer::Deny => false,
                    Answer::Once => true,
                    Answer::Session => {
                        if let Some(token) = token {
                            self.grants.grant(&token);
                        }
                        true
                    }
                },
            },
        }
    }

    /// Whether [`Self::approve`] would put this call to the operator, as the
    /// mode and the store stand now. Only a real ask counts: a table that says
    /// "ask" with nobody to ask refuses instead, and reads as no ask here, so a
    /// permission hook never stands in front of a decision the operator was not
    /// going to be asked about.
    pub(crate) fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        matches!(self.decide(tool, arguments), Verdict::Ask { .. }) && self.prompt.is_some()
    }
}
