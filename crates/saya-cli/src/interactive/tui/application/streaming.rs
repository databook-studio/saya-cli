//! Agent streaming lifecycle.

use super::super::agent::{self, StreamMsg};
use super::super::stream_events::apply_event;
use super::super::transcript::BlockKind;
use super::super::types::{App, PendingApproval};
use crate::interactive::session_state::SessionState;
use saya_agent::AgentEvent;

impl App {
    /// Starts streaming an agent prompt on a background thread.
    pub(crate) fn start_agent(&mut self, prompt: String, state: &SessionState) {
        let approval = state
            .approval_mode
            .parse()
            .unwrap_or(saya_agent::ApprovalPolicy::Ask);
        self.stream = Some(agent::start(
            self.runtime.clone(),
            prompt,
            approval,
            state.prompt_overrides(),
            state.provider_history(),
            self.state_db.clone(),
        ));
        self.stream_started = Some(std::time::Instant::now());
    }

    /// Drains any queued agent-stream messages into the transcript. Returns true
    /// when the request just finished (so the caller can persist the session).
    pub(crate) fn drain_stream(&mut self, state: &mut SessionState) -> bool {
        let mut messages = Vec::new();
        if let Some(stream) = self.stream.as_mut() {
            while let Ok(msg) = stream.rx.try_recv() {
                messages.push(msg);
            }
        }
        let prompt = self
            .stream
            .as_ref()
            .map(|s| s.prompt.clone())
            .unwrap_or_default();
        let mut finished = false;
        for msg in messages {
            match msg {
                StreamMsg::Event(event) => {
                    match &event {
                        AgentEvent::ToolRequested { name, .. } => {
                            self.activity = Some(name.clone());
                        }
                        AgentEvent::AssistantText { .. } | AgentEvent::ToolCompleted { .. } => {
                            self.activity = None;
                        }
                        _ => {}
                    }
                    apply_event(&mut self.transcript, event);
                }
                StreamMsg::ApprovalRequest {
                    tool,
                    detail,
                    respond,
                } => {
                    self.pending_approval = Some(PendingApproval {
                        tool,
                        detail,
                        respond,
                    });
                }
                StreamMsg::Done(result) => {
                    match result {
                        Ok(output) => state.record_turn(
                            prompt.clone(),
                            output.answer.clone(),
                            output.used_bounded_sql_query,
                            output.tool_metadata.clone(),
                        ),
                        Err(error) => self.transcript.push(BlockKind::Error, error),
                    }
                    finished = true;
                }
            }
        }
        if finished {
            self.stream = None;
            self.stream_started = None;
            self.activity = None;
        }
        // No forced scroll: when the user is at the bottom the newest lines show
        // automatically; when they've scrolled up to read, streaming leaves them.
        finished
    }
}
