//! The end of a turn: recording it, folding in usage, and telling the user
//! whether it finished, failed, or stopped because they asked.

use super::super::super::transcript::BlockKind;
use super::super::super::types::App;
use super::super::super::usage_footer;
use crate::interactive::session_state::SessionState;
use saya_agent::AgentOutput;

impl App {
    /// Handles a settled stream: the `Done` arm of the drain loop, lifted out
    /// so `drain.rs` stays under the size cap.
    pub(super) fn settle_done(
        &mut self,
        result: Result<AgentOutput, String>,
        state: &mut SessionState,
        prompt: String,
    ) {
        // Whether the user asked this stream to stop: Esc fires the
        // token before the worker's settle arrives. Read before
        // the match borrows `result`, while the stream exists;
        // the finish block clears it after the batch.
        let stop_confirmed = self
            .request
            .stream
            .as_ref()
            .is_some_and(|stream| stream.cancel.is_cancelled());
        // Whether this run continued after the provider capped a
        // response mid-answer: the loop re-instructs, the partial
        // stays discarded, and the resume anchors live in earlier
        // tool results. A finished turn that continued is not a
        // turn boundary for compaction — compacting there risks
        // the very lines the resume needs. `TurnReset` (retried
        // transport attempts) re-streams the same answer and is
        // not a continuation; only the output-cap re-instruction
        // counts. `output.truncated` is that signal: it is set
        // only when a run salvaged an answer after a ceiling ran
        // out, never on a natural completion.
        let continued = result
            .as_ref()
            .map(|output| output.truncated)
            .unwrap_or(false);
        match result {
            Ok(output) => {
                state.task_list = self.session.tasks().current();
                state.record_turn(
                    prompt.clone(),
                    output.answer.clone(),
                    output.used_bounded_sql_query,
                    output.tool_metadata.clone(),
                );
                let usage = &output.usage;
                // Accumulate into the session total before the
                // footer is built, so its session segment
                // includes the turn it reports. `record` applies
                // the same zero-guard as the push below, so a
                // silent provider's all-zero usage adds nothing.
                state.usage.record(usage);
                if usage.input_tokens > 0 || usage.output_tokens > 0 {
                    // The user-declared window is a fact about
                    // this deployment and wins over the table;
                    // otherwise the table answers for the live
                    // model, and `None` for a model it does not
                    // know stays absent in the footer.
                    let window = self
                        .runtime
                        .resolved
                        .ai
                        .context_window_tokens
                        .or_else(|| saya_config::context_window_tokens(&state.model));
                    self.transcript.push(
                        BlockKind::System,
                        usage_footer::transcript_footer(
                            usage,
                            &state.usage.answering,
                            self.request.last_answering_input,
                            window,
                        ),
                    );
                    // The one-shot context warning: fire on the
                    // upward crossing of the warn threshold, stay
                    // silent while above it, re-arm below it. No
                    // window (or no per-call report) means no
                    // percentage, so the flag is untouched —
                    // absence is not zero. `off` silences it
                    // along with the automatic trigger.
                    if crate::interactive::auto_compact::warning_enabled(
                        self.runtime.resolved.ai.compaction,
                    ) && let Some(percent) = saya_agent::context_utilisation_percent(
                        self.request.last_answering_input,
                        window,
                    ) {
                        if percent >= saya_agent::CONTEXT_WARN_PERCENT {
                            if !state.context_warned {
                                state.context_warned = true;
                                if let Some(window) = window
                                    && let Some(notice) =
                                        usage_footer::context_warn_notice(percent, window)
                                {
                                    self.transcript.push(BlockKind::System, notice);
                                }
                            }
                        } else {
                            state.context_warned = false;
                        }
                    }
                    // The automatic trigger: the same numerator
                    // and window as the warning and the footer —
                    // no second measurement — read at the turn
                    // boundary only, when no continuation is
                    // outstanding and no compaction is running.
                    // Unknown window or no report never fires.
                    // Capture the values before the finish block
                    // below clears the per-turn numerator.
                    let auto_input = self.request.last_answering_input;
                    let fire = crate::interactive::auto_compact::should_auto_compact(
                        &crate::interactive::auto_compact::AutoCompactInput {
                            mode: self.runtime.resolved.ai.compaction,
                            answering_input: auto_input,
                            window,
                            continued,
                            compact_running: self.compact_task.is_some(),
                            auto_failed: state.auto_compact_failed,
                        },
                    );
                    if fire {
                        crate::interactive::compact_task::start_automatic(self, state);
                    }
                }
                // Fold the extraction call's usage into a separate
                // learning total. `learning_usage` is `None` when
                // no extraction ran or it produced no response, so
                // a session with learning disabled records nothing
                // here — the answering total is unchanged.
                state.usage.record_learning(output.learning_usage);
            }
            Err(error) => {
                // The stop's confirmation, not a failure: the
                // token says this app asked, the worker's settle
                // text says the worker confirmed — kept work
                // stays kept — so it renders as a System block.
                // Both must agree: a genuine failure arriving
                // with the token fired keeps its error block.
                if stop_confirmed && error == "request cancelled" {
                    self.transcript
                        .push(BlockKind::System, "Stopped. Completed work is kept.");
                } else {
                    self.transcript.push(BlockKind::Error, error);
                }
            }
        }
    }
}
