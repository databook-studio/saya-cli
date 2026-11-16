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

/// Runs `/compact` to completion: plan, summarise, validate, apply or leave
/// the session exactly as it was. Pure against `state` except on validated
/// success; the transcript is never rewritten.
///
/// `automatic` names the trigger only — the operation is identical either way
/// (one code path, two triggers), and the message stays the manual one: the
/// poll path prefixes the automatic strings, beside the single operation.
pub(crate) async fn run(
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    _session: &SessionUniverse,
    metrics: Option<&LocalMetrics>,
) -> CompactResult {
    run_with_trigger(runtime, state, _session, false, metrics).await
}

/// The shared operation with the trigger named: [`run`] is the manual
/// spelling, the automatic path passes `automatic: true`. Kept as one
/// function — never two copies — so the paths cannot drift.
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
    let Some(plan) = manual::plan(&state.turns, &history) else {
        return no_work("Nothing to compact: the conversation is short enough already.".into());
    };
    if state.turns.len() <= COMPACT_MIN_TURNS {
        return no_work("Nothing to compact: the conversation is short enough already.".into());
    }
    let overrides = state.prompt_overrides();
    let ai = crate::agent::runtime::effective_ai(&runtime.resolved.ai, &overrides);
    let resolver = runtime.secret_resolver();
    let provider = match provider::build(&ai, &resolver) {
        Ok(provider) => provider,
        Err(error) => {
            return failed(
                manual::failure_message(&format!("the summariser errored: {error}")),
                None,
            );
        }
    };
    let started = std::time::Instant::now();
    if let Some(metrics) = metrics {
        metrics.record(&saya_agent::AgentMetric::Count(
            saya_agent::AgentMetricCount::CompactionStarted,
        ));
    }
    let result = match super::provider::summarise(&*provider, &ai.model, &plan).await {
        Ok(outcome) => {
            match manual::apply_with_continuation(state, &plan, &outcome.summary, &digest_of) {
                Ok(()) => CompactResult {
                    message: manual::success_message(plan.compacted_turns, &outcome.summary),
                    failed: false,
                    usage: outcome.usage,
                    summary: Some(outcome.summary),
                    compacted_turns: plan.compacted_turns,
                    source_identity: Some(plan.source_identity),
                    automatic,
                },
                Err(reason) => failed(manual::failure_message(&reason), outcome.usage),
            }
        }
        Err(reason) => failed(manual::failure_message(&reason), None),
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
