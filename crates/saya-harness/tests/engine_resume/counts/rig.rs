//! The family rig: the bounded step a plan carries, the resume inputs a
//! crashed run hands to the public entry point, and the journal query
//! that extracts a step's count rows.

use super::stubs::ScriptedAnswerProvider;
use super::*;
use saya_agent::AgentEventSink;
use saya_types::Budgets;

pub(super) fn bounded_step(goal: &str, turns: u64, tool_calls: Option<u64>) -> StepSpec {
    let mut budget = Budgets::default();
    budget.turns = Some(turns);
    budget.tool_calls = tool_calls;
    StepSpec::new(
        goal,
        Capabilities::default(),
        Some(budget),
        Vec::new(),
        None,
    )
    .unwrap()
}

pub(super) fn inputs<'a>(
    run: &'a CrashedRun,
    provider: &'a ScriptedAnswerProvider,
    agent_stream: Option<Arc<dyn AgentEventSink>>,
) -> ResumeRun<'a> {
    ResumeRun {
        run_id: run.run_id.clone(),
        store: run.store.clone(),
        plan: run.plan.clone(),
        incomplete_policy: saya_harness::engine::IncompletePolicy::Conservative,
        workspace: workspace(&run.root),
        collaborators: EpisodeCollaborators {
            provider,
            approval: &run.stubs.approval,
            toolsets: &run.toolsets,
            cancellation: CancellationToken::default(),
        },
        request: request(),
        bounds: bounds(),
        wall_clock: None,
        token_ceiling: None,
        download_budget: None,
        journal_wire: None,
        agent_stream,
    }
}

pub(super) fn count_rows_for_step(
    events: &[RunEvent],
    step: usize,
) -> Vec<(Option<u64>, Option<u64>)> {
    let Some(start) = events
        .iter()
        .position(|event| *event == RunEvent::StepStarted { step })
    else {
        return Vec::new();
    };
    events[start + 1..]
        .iter()
        .take_while(|event| !matches!(event, RunEvent::StepStarted { step: next } if *next != step))
        .filter_map(|event| match event {
            RunEvent::Usage {
                tokens: None,
                turns,
                tool_calls,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
                ..
            } if turns.is_some() || tool_calls.is_some() => Some((*turns, *tool_calls)),
            _ => None,
        })
        .collect()
}
