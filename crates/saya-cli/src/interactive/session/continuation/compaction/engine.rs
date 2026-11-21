/// The outcome of one `/compact` invocation: the message to show, and the
/// compaction call's usage (kept apart from the answering total). On success
/// the summary and compacted-turn count ride along so the TUI poll path can
/// apply the same working-memory change to the live session the worker
/// applied to its clone — one operation, two triggers, byte-identical.
pub(crate) struct CompactResult {
    pub(crate) message: String,
    pub(crate) failed: bool,
    pub(crate) usage: Option<saya_agent::TokenUsage>,
    pub(crate) summary: Option<String>,
    pub(crate) compacted_turns: usize,
    pub(crate) source_identity: Option<[u8; 32]>,
    /// `true` when the worker ran as an automatic firing: the poll path
    /// prefixes the manual strings and applies the no-retry failure policy.
    pub(crate) automatic: bool,
}

/// The shared operation with the trigger named: the automatic path passes
/// `automatic: true`, the operator's ask `false`. One function — never two
/// copies — so the paths cannot drift.
pub(crate) async fn run_with_trigger(
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    session: &SessionUniverse,
    automatic: bool,
    metrics: Option<&LocalMetrics>,
) -> CompactResult {
    let digest_of = digest_resolver(session.workspace());
    let no_work = |message: String| {
        record_compaction(metrics, saya_agent::AgentMetricOutcome::Skipped, 0);
        CompactResult {
            message,
            failed: false,
            usage: None,
            summary: None,
            compacted_turns: 0,
            source_identity: None,
            automatic,
        }
    };
    let failed = |message: String, usage: Option<saya_agent::TokenUsage>| CompactResult {
        message,
        failed: true,
        usage,
        summary: None,
        compacted_turns: 0,
        source_identity: None,
        automatic,
    };
    let history = state.provider_history();
    if state.turns.len() <= COMPACT_MIN_TURNS {
        return no_work("Nothing to compact: the conversation is short enough already.".into());
    }
    let overrides = state.prompt_overrides();
    let started = std::time::Instant::now();
    let result = match summarise(runtime, &overrides, &state.turns, &history, metrics).await {
        Pass::Nothing => {
            return no_work("Nothing to compact: the conversation is short enough already.".into());
        }
        Pass::Failed { reason, usage } => failed(manual::failure_message(&reason), usage),
        Pass::Summarised {
            plan,
            summary,
            usage,
        } => match manual::apply_with_continuation(state, &plan, &summary, &digest_of) {
            Ok(()) => CompactResult {
                message: manual::success_message(plan.compacted_turns, &summary),
                failed: false,
                usage,
                summary: Some(summary),
                compacted_turns: plan.compacted_turns,
                source_identity: Some(plan.source_identity),
                automatic,
            },
            Err(reason) => failed(manual::failure_message(&reason), usage),
        },
    };
    record_compaction(
        metrics,
        if result.failed {
            saya_agent::AgentMetricOutcome::Failed
        } else {
            saya_agent::AgentMetricOutcome::Succeeded
        },
        u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
    );
    result
}

/// What one summarising pass produced, before anyone applies it: the plan and
/// its validated summary, a reason it failed, or nothing to do.
pub(crate) enum Pass {
    /// The conversation is short enough already: no provider call ran.
    Nothing,
    /// A validated summary of `plan.compacted_turns` turns.
    Summarised {
        plan: manual::CompactionPlan,
        summary: String,
        usage: Option<saya_agent::TokenUsage>,
    },
    /// The reason it failed, in words a person reads.
    Failed {
        reason: String,
        usage: Option<saya_agent::TokenUsage>,
    },
}

/// Plans one compaction and asks the summariser for it: the one operation
/// behind every trigger — the manual `/compact`, the automatic firing, and a
/// chat session's compaction at the turn boundary.
///
/// The conversation is stated as the caller holds it — the turns and the
/// history replayed for them — so an owner whose conversation is not a
/// `SessionState` runs the same plan, the same summariser call and the same
/// validation, and never a copy of them. Applying the summary is the caller's:
/// this half changes nothing.
pub(crate) async fn summarise(
    runtime: &RuntimeConfig,
    overrides: &crate::agent::runtime::PromptOverrides,
    turns: &[saya_store::RedactedTurn],
    history: &[saya_agent::ChatMessage],
    metrics: Option<&LocalMetrics>,
) -> Pass {
    let Some(plan) = manual::plan(turns, history) else {
        return Pass::Nothing;
    };
    let ai = crate::agent::runtime::effective_ai(&runtime.resolved.ai, overrides);
    let resolver = runtime.secret_resolver();
    let provider = match provider::build(&ai, &resolver) {
        Ok(provider) => provider,
        Err(error) => {
            return Pass::Failed {
                reason: format!("the summariser errored: {error}"),
                usage: None,
            };
        }
    };
    if let Some(metrics) = metrics {
        metrics.record(&saya_agent::AgentMetric::Count(
            saya_agent::AgentMetricCount::CompactionStarted,
        ));
    }
    match super::provider::summarise(&*provider, &ai.model, &plan).await {
        Ok(outcome) => Pass::Summarised {
            plan,
            summary: outcome.summary,
            usage: outcome.usage,
        },
        Err(reason) => Pass::Failed {
            reason,
            usage: None,
        },
    }
}

/// Records one Compaction phase sample through the session collector, when
/// one rides the call: the measured work window (summariser call through
/// validated apply) for a real run, a constant zero for a no-work result.
fn record_compaction(
    metrics: Option<&LocalMetrics>,
    outcome: saya_agent::AgentMetricOutcome,
    elapsed_ns: u64,
) {
    if let Some(metrics) = metrics {
        metrics.record(&saya_agent::AgentMetric::Phase {
            phase: saya_agent::AgentMetricPhase::Compaction,
            elapsed_ns,
            outcome,
        });
    }
}
use super::super::digest_resolver;
use super::manual::{self, COMPACT_MIN_TURNS};
use crate::agent::provider;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::state::SessionState;
use crate::interactive::session::universe::SessionUniverse;
use crate::local_metrics::LocalMetrics;
