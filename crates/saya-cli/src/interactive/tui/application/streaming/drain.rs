//! Agent streaming lifecycle.

use super::super::super::agent::StreamMsg;
use super::super::super::stream_events::apply_event;
use super::super::super::transcript::BlockKind;
use super::super::super::types::{App, LastQuery, PendingApproval};
use crate::interactive::session_state::SessionState;
use saya_agent::{AgentEvent, UsageCall};

impl App {
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
                        AgentEvent::ToolRequested {
                            name, arguments, ..
                        } => {
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
                        // The failed attempt's partial answer was discarded
                        // and the turn is retrying; the spinner falls back to
                        // plain thinking instead of a stale tool label until
                        // the re-streamed answer arrives.
                        AgentEvent::TurnReset => {
                            self.request.activity = None;
                        }
                        // The answer has streamed but the turn is not over:
                        // extraction is a second provider call the loop awaits.
                        // Without this the status bar falls back to "thinking"
                        // beside a finished answer, which reads as a hang.
                        AgentEvent::KnowledgeLearningStarted => {
                            self.request.activity = Some("learning".into());
                        }
                        // The last answering call's input is the freshest context-size
                        // figure the provider gave — the numerator for the
                        // footer's utilisation. The extraction call's prompt
                        // is not the conversation, so it never updates this.
                        AgentEvent::Usage {
                            call: UsageCall::Answer,
                            usage,
                        } => {
                            self.request.last_answering_input = Some(usage.input_tokens);
                        }
                        _ => {}
                    }
                    apply_event(&mut self.transcript, event, state.show_thinking);
                }
                StreamMsg::Notice(message) => {
                    // A system fact the decider said — today, that the
                    // session journal could not record a grant the user
                    // just made. The consent stands; the missing audit
                    // line must not be silent.
                    self.transcript.push(BlockKind::System, message);
                }
                StreamMsg::ApprovalRequest {
                    tool,
                    detail,
                    grant,
                    respond,
                } => {
                    self.request.pending_approval = Some(PendingApproval {
                        tool,
                        detail,
                        grant,
                        respond,
                    });
                }
                StreamMsg::Done(result) => {
                    self.settle_done(result, state, prompt.clone());
                    finished = true;
                }
            }
        }
        if finished {
            self.request.stream = None;
            self.request.started = None;
            self.request.activity = None;
            // The numerator belongs to the turn that reported it; the next
            // turn must not show this one's figure if the provider goes
            // silent.
            self.request.last_answering_input = None;
        }
        // No forced scroll: when the user is at the bottom the newest lines show
        // automatically; when they've scrolled up to read, streaming leaves them.
        finished
    }
}
