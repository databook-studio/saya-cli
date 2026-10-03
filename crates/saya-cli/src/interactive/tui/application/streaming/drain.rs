//! Agent streaming lifecycle.

use super::super::super::agent::StreamMsg;
use super::super::super::stream_events::apply_event;
use super::super::super::transcript::BlockKind;
use super::super::super::types::{
    App, LastQuery, MAX_PENDING_QUERIES, PendingApproval, PendingQuery,
};
use crate::interactive::session_state::SessionState;
use crate::interactive::tui::capture_agent::{AgentCaptureOutcome, promote_agent_capture};
use crate::render::tool_groups::is_failure_summary;
use saya_agent::{AgentEvent, ProviderRecoveryPhase, UsageCall};

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
                    // The capture a successful completion pairs, if any; its
                    // provenance line is pushed after the event has rendered.
                    let mut promoted = None;
                    match &event {
                        AgentEvent::ToolRequested {
                            name, arguments, ..
                        } => {
                            self.request.activity = Some(name.clone());
                            // A request is a *candidate*, never a selectable
                            // query: it waits on the bounded FIFO until its
                            // completion decides (see the ToolCompleted arm).
                            // Fan-out (`bounded_sql_query_all`) is not a
                            // concrete query — the user picks one connection
                            // with /sql — so it never enters the FIFO.
                            if name == "bounded_sql_query"
                                && let Some(sql) = arguments.get("sql").and_then(|v| v.as_str())
                                && !self.pending_queries_desync
                            {
                                // The connection that actually runs an unnamed
                                // query is the session's one profile, read at
                                // request time (a later /connect must not
                                // retag it); when the model named one, that
                                // name is the fact and stands as stated.
                                let connection = arguments
                                    .get("connection")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(str::to_string)
                                    .or_else(|| session_connection(state, self));
                                self.pending_queries.push_back(PendingQuery {
                                    sql: sql.to_string(),
                                    connection,
                                });
                                // Bounded: a runaway agent cannot grow this
                                // past the cap. Beyond it the pairing is
                                // already broken — dropping the oldest
                                // candidate would make every later completion
                                // pop one position late, promoting a failed
                                // query's SQL — so the whole FIFO is
                                // discarded and marked desynchronised: for
                                // the rest of the turn no completion promotes
                                // anything and no candidate is queued. Both
                                // reset when the turn ends.
                                if self.pending_queries.len() > MAX_PENDING_QUERIES {
                                    self.pending_queries.clear();
                                    self.pending_queries_desync = true;
                                    // The pairing is off for the rest of the
                                    // turn: no queued capture can be paired
                                    // with its completion either.
                                    self.agent_captures.clear_queue();
                                }
                            }
                        }
                        AgentEvent::AssistantText { .. } => {
                            self.request.activity = None;
                        }
                        AgentEvent::ToolCompleted { name, summary } => {
                            self.request.activity = None;
                            // The completion decides promotion, classified by
                            // the same failure predicate the renderers use —
                            // the selectable query and the transcript can
                            // never disagree. A failure (or an unmatched
                            // completion, when the FIFO is empty) leaves
                            // `last_query` untouched. After an overflow the
                            // FIFO is desynchronised for the rest of the
                            // turn: completions can no longer be paired with
                            // their requests, so none promotes anything and
                            // `last_query` keeps its previous value.
                            if name == "bounded_sql_query"
                                && !self.pending_queries_desync
                                && let Some(pending) = self.pending_queries.pop_front()
                                && !is_failure_summary(summary)
                            {
                                // D12/C2 pairing: THIS query's capture decides
                                // the slot — held, refused (cleared, gap
                                // named), or absent (cleared). No silent
                                // fallback to an older result.
                                promoted = promote_agent_capture(
                                    &pending,
                                    &mut self.agent_captures,
                                    &mut self.captured,
                                );
                                self.last_query = Some(LastQuery {
                                    sql: pending.sql,
                                    connection: pending.connection,
                                });
                            }
                        }
                        AgentEvent::ToolDenied { name, .. } => {
                            // A denied call never ran: its candidate is
                            // consumed without promoting. The activity
                            // indicator is untouched, as before. While the
                            // FIFO is desynchronised nothing is popped: the
                            // pairing is not trusted for the rest of the turn.
                            if name == "bounded_sql_query" && !self.pending_queries_desync {
                                self.pending_queries.pop_front();
                            }
                        }
                        // The failed attempt's partial answer was discarded
                        // and the turn is retrying: the spinner falls back
                        // to plain thinking instead of a stale tool label,
                        // and the discarded attempt's pairing state resets
                        // as at turn end, so the retry pairs only its own.
                        AgentEvent::TurnReset => {
                            if !self
                                .request
                                .activity
                                .as_deref()
                                .is_some_and(|activity| activity.starts_with("retrying provider"))
                            {
                                self.request.activity = None;
                            }
                            self.pending_queries.clear();
                            self.pending_queries_desync = false;
                            self.agent_captures.clear_queue();
                        }
                        AgentEvent::ProviderRecovery {
                            phase: ProviderRecoveryPhase::Retrying,
                            attempt,
                            limit,
                            ..
                        } => {
                            self.request.activity =
                                Some(format!("retrying provider (attempt {attempt} of {limit})"));
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
                    // A promoted capture's provenance is visible: source,
                    // connection, rows, short id, model-limited scope.
                    if let Some(evidence) = promoted {
                        self.transcript
                            .push(BlockKind::System, evidence.human_line());
                    }
                }
                StreamMsg::QueryCaptured(capture) => {
                    // The typed result of one successful agent query (D12),
                    // queued for the pairing below. Skipped while the FIFO is
                    // desynchronised: nothing pairs from an untrusted turn.
                    if !self.pending_queries_desync {
                        self.agent_captures
                            .push(AgentCaptureOutcome::Captured(capture));
                    }
                }
                StreamMsg::QueryCaptureRefused {
                    sql,
                    connection,
                    reason,
                } => {
                    // The query ran but the capture was refused — the model's
                    // view was truncated or redacted, or the result is over
                    // the capture budget: queued so its completion pairs,
                    // clears the slot, and names the reason (R3).
                    if !self.pending_queries_desync {
                        self.agent_captures.push(AgentCaptureOutcome::Refused {
                            sql,
                            connection,
                            reason,
                        });
                    }
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
                        // Every fresh approval opens at the top of its fact
                        // body: the offset is per-pending-approval state.
                        scroll: 0,
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
            // The turn is over: no pending candidate may survive into the
            // next turn, where it could be promoted by an unmatched
            // completion. The desynchronised mark resets with the FIFO:
            // the next turn pairs from a trusted FIFO again.
            self.pending_queries.clear();
            self.pending_queries_desync = false;
            // No unmatched capture survives the turn either: its completion
            // never came, and pairing it later would attach a foreign result.
            self.agent_captures.clear_queue();
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

/// The connection an unnamed agent `bounded_sql_query` runs on: the session's
/// active profile, or — when none is selected — the profile the session runs
/// on (the runtime's resolved default). Both are readable here, on `App`, at
/// request time; an unnamed query promoted into `LastQuery` must carry it so
/// `/investigation save` can save "the connection that actually ran it".
fn session_connection(state: &SessionState, app: &App) -> Option<String> {
    state
        .profile
        .clone()
        .or_else(|| app.runtime.resolved.profile_name.clone())
}
