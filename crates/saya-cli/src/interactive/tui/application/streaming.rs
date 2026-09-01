//! Agent streaming lifecycle.

use super::super::agent::{self, StreamMsg};
use super::super::stream_events::apply_event;
use super::super::transcript::BlockKind;
use super::super::types::{App, LastQuery, PendingApproval};
use crate::interactive::session_state::SessionState;
use saya_agent::AgentEvent;

impl App {
    /// Starts streaming an agent prompt on a background thread.
    pub(crate) fn start_agent(&mut self, prompt: String, state: &SessionState) {
        let approval = state
            .approval_mode
            .parse()
            .unwrap_or(saya_agent::ApprovalPolicy::Ask);
        self.request.stream = Some(agent::start(
            self.runtime.clone(),
            prompt,
            approval,
            state.prompt_overrides(),
            state.provider_history(),
            self.state_db.clone(),
            self.last_query.as_ref().map(|lq| lq.sql.clone()),
        ));
        self.request.started = Some(std::time::Instant::now());
    }

    /// Drains any queued agent-stream messages into the transcript. Returns true
    /// when the request just finished (so the caller can persist the session).
    pub(crate) fn drain_stream(&mut self, state: &mut SessionState) -> bool {
        let mut messages = Vec::new();
        if let Some(stream) = self.request.stream.as_mut() {
            while let Ok(msg) = stream.rx.try_recv() {
                messages.push(msg);
            }
        }
        let prompt = self
            .request
            .stream
            .as_ref()
            .map(|s| s.prompt.clone())
            .unwrap_or_default();
        let mut finished = false;
        for msg in messages {
            match msg {
                StreamMsg::Event(event) => {
                    match &event {
                        AgentEvent::ToolRequested { name, arguments } => {
                            self.request.activity = Some(name.clone());
                            if matches!(
                                name.as_str(),
                                "bounded_sql_query" | "bounded_sql_query_all"
                            ) && let Some(sql) = arguments.get("sql").and_then(|v| v.as_str())
                            {
                                let connection = arguments
                                    .get("connection")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(str::to_string);
                                self.last_query = Some(LastQuery {
                                    sql: sql.to_string(),
                                    connection,
                                });
                            }
                        }
                        AgentEvent::AssistantText { .. } | AgentEvent::ToolCompleted { .. } => {
                            self.request.activity = None;
                        }
                        // The answer has streamed but the turn is not over:
                        // extraction is a second provider call the loop awaits.
                        // Without this the status bar falls back to "thinking"
                        // beside a finished answer, which reads as a hang.
                        AgentEvent::KnowledgeLearningStarted => {
                            self.request.activity = Some("learning".into());
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
                    self.request.pending_approval = Some(PendingApproval {
                        tool,
                        detail,
                        respond,
                    });
                }
                StreamMsg::Done(result) => {
                    match result {
                        Ok(output) => {
                            state.record_turn(
                                prompt.clone(),
                                output.answer.clone(),
                                output.used_bounded_sql_query,
                                output.tool_metadata.clone(),
                            );
                            let usage = &output.usage;
                            if usage.input_tokens > 0 || usage.output_tokens > 0 {
                                self.transcript.push(
                                    BlockKind::System,
                                    format!(
                                        "{} tokens in · {} tokens out",
                                        usage.input_tokens, usage.output_tokens
                                    ),
                                );
                            }
                            // Accumulate into the session total. `record`
                            // applies the same zero-guard (invariant 4), so a
                            // silent provider's all-zero usage adds nothing.
                            state.usage.record(usage);
                        }
                        Err(error) => self.transcript.push(BlockKind::Error, error),
                    }
                    finished = true;
                }
            }
        }
        if finished {
            self.request.stream = None;
            self.request.started = None;
            self.request.activity = None;
        }
        // No forced scroll: when the user is at the bottom the newest lines show
        // automatically; when they've scrolled up to read, streaming leaves them.
        finished
    }
}
