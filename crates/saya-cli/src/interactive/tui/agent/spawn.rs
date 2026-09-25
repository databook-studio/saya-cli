//! Spawning the agent onto a background thread.

use super::approval::{ChannelApproval, ChannelSink, StreamRequest, approval_capabilities};
use super::messages::{Stream, StreamMsg};
use crate::agent::runtime::run_prompt_with_sink;
use saya_agent::{ApprovalDecider, CancellationToken};
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

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
    } = request;
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
        let result = runtime_handle.block_on(run_prompt_with_sink(
            runtime.as_ref(),
            &prompt_worker,
            approval,
            approval_capabilities().0, // never read stdin: the modal collects approvals
            approval_capabilities().1, // the modal is the approval surface
            overrides,
            history,
            &sink,
            cancel_worker,
            Some(state_db),
            Some(decider),
            last_sql,
            Some(session),
            agent_mode,
        ));
        let _ = tx.send(StreamMsg::Done(result.map_err(|error| error.to_string())));
    });

    Stream { rx, cancel, prompt }
}
