//! Spawning the agent onto a background thread.

use super::approval::{ChannelApproval, ChannelSink, StreamRequest, approval_capabilities};
use super::messages::{Stream, StreamMsg};
use crate::agent::tools::{CaptureEvent, CaptureHook};
use saya_agent::{ApprovalDecider, CancellationToken};
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// The TUI's capture hook, built from the turn's channel sender: it forwards
/// one successful agent query's typed result — or its refusal with the reason
/// the snapshot names (R3) — onto the same channel the turn's events ride, so
/// the message precedes the loop's `ToolCompleted` for the same call.
pub(crate) fn capture_hook(tx: UnboundedSender<StreamMsg>) -> CaptureHook {
    Arc::new(move |event| {
        let message = match event {
            CaptureEvent::Captured(capture) => StreamMsg::QueryCaptured(capture),
            CaptureEvent::Refused {
                sql,
                connection,
                reason,
            } => StreamMsg::QueryCaptureRefused {
                sql,
                connection,
                reason,
            },
        };
        let _ = tx.send(message);
    })
}

/// Spawns the agent on a background thread and returns the live stream handle.
pub(crate) fn start(request: StreamRequest) -> Stream {
    let StreamRequest {
        runtime,
        prompt,
        approval,
        policy,
        overrides,
        history,
        state_db,
        last_sql,
        session,
        journal,
        agent_mode,
        prior_tool_outcomes,
        compaction_summary,
    } = request;
    let prior_tool_outcomes = (!history.is_empty())
        .then_some(prior_tool_outcomes)
        .flatten();
    let (tx, rx) = unbounded_channel();
    let cancel = CancellationToken::new();
    let cancel_worker = cancel.clone();
    let prompt_worker = prompt.clone();
    // The turn's primary handle rides the session's universe: the decider
    // holds a clone, and the turn binds the registry's primary into it.
    let primary = session.primary.clone();
    // The prompt facts come from the members this session actually composed,
    // read off the universe and the resolved config — the modal states only
    // these.
    let facts = session.approval_facts(&runtime);
    // The capture hook rides the same channel: a successful agent query
    // hands the TUI the typed result the model saw (C1; the pairing is C2).
    let capture = capture_hook(tx.clone());

    std::thread::spawn(move || {
        let sink = ChannelSink { tx: tx.clone() };
        let decider: Arc<dyn ApprovalDecider> = Arc::new(ChannelApproval::new(
            tx.clone(),
            policy,
            primary,
            facts,
            journal,
        ));
        let runtime_handle = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(handle) => handle,
            Err(error) => {
                let _ = tx.send(StreamMsg::Done(Err(error.to_string())));
                return;
            }
        };
        let result =
            runtime_handle.block_on(crate::agent::runtime::run_prompt_with_sink_and_outcomes(
                crate::agent::runtime::SinkPrompt {
                    runtime: runtime.as_ref(),
                    prompt: &prompt_worker,
                    approval,
                    can_prompt: approval_capabilities().0,
                    can_obtain_approval: approval_capabilities().1,
                    overrides,
                    history,
                    sink: &sink,
                    cancellation: cancel_worker,
                    state_db: Some(state_db),
                    decider: Some(decider),
                    last_sql,
                    session: Some(session),
                    agent_mode,
                    capture: Some(capture),
                    prior_tool_outcomes,
                    compaction_summary,
                },
            ));
        let _ = tx.send(StreamMsg::Done(result.map_err(|error| error.to_string())));
    });

    Stream { rx, cancel, prompt }
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
