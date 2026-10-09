/// Maps one agent event to what the headless renderer should print, or `None`
/// when the event is a pure progress signal with nothing to say here.
///
/// The `None` case exists because the catch-all below is deliberately loud: a
/// variant this renderer does not understand must surface as
/// `NotImplemented` rather than silently end the stream. That is right for an
/// event carrying content, and wrong for one that carries none — a spinner
/// label is meaningful to the TUI and meaningless to a pipe. Without this
/// distinction, adding a progress event prints `unrecognized agent event` at
/// the user, which has now happened three times (see the arms below).
pub(crate) fn terminal_event(event: AgentEvent) -> Option<TerminalEvent> {
    Some(match event {
        AgentEvent::AssistantText { text } => TerminalEvent::AssistantText { text },
        AgentEvent::ToolRequested {
            name,
            arguments,
            effect,
        } => {
            let detail = crate::agent::tools::tool_call_detail(&name, &arguments);
            TerminalEvent::ToolRequested {
                name,
                detail,
                effect,
            }
        }
        AgentEvent::ToolCompleted { name, summary } => {
            TerminalEvent::ToolCompleted { name, summary }
        }
        AgentEvent::ToolDenied { name, reason } => TerminalEvent::ToolDenied { name, reason },
        AgentEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => TerminalEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        },
        AgentEvent::KnowledgeOverridden { findings } => {
            TerminalEvent::KnowledgeOverridden { findings }
        }
        // Extraction timed out or errored after the turn succeeded — surface it
        // rather than fall through to the `unrecognized agent event` catch-all.
        AgentEvent::KnowledgeLearningSkipped { reason } => {
            TerminalEvent::KnowledgeLearningSkipped { reason }
        }
        // The extraction circuit breaker tripped — surface it the same way,
        // rather than fall through to the `unrecognized agent event` catch-all.
        AgentEvent::KnowledgeLearningDisabled { model, misses } => {
            TerminalEvent::KnowledgeLearningDisabled { model, misses }
        }
        // Learning used to fall through to the catch-all below and print
        // `unrecognized agent event` — an error string at the exact moment the
        // product did the thing it is for.
        AgentEvent::KnowledgeProposed { claim } => TerminalEvent::KnowledgeLearned { claim },
        // Progress only: the TUI labels its spinner with this, and a pipe has
        // no spinner to label. Dropped rather than rendered — not forgotten,
        // which is what the catch-all would make of it.
        AgentEvent::KnowledgeLearningStarted => return None,
        // A provider attempt began. The TUI records a rollback watermark here;
        // a pipe has nothing to roll back and nothing to say. Explicitly None
        // rather than left to the catch-all, which would print `unrecognized
        // agent event` at the user — the regression the note above says has
        // already shipped three times for contentless variants.
        AgentEvent::TurnStarted => return None,
        // The model's chain-of-thought. This is the one case where rendering to
        // nothing is a *scope* decision rather than a *nature-of-the-event*
        // decision: reasoning is content (it mirrors `AssistantText`), so by its
        // nature it would belong on the loud path below — but display belongs
        // to the interactive transcript, not a pipe, and nothing is displayed
        // by default. So it renders to `None` here, the same way a progress
        // signal does, for a different reason. The tests pin both halves: this
        // arm stays silent, and a content event (`AssistantText`, `Complete`)
        // still reaches the loud path — so a future reader cannot conclude
        // reasoning is progress, and a future change cannot silence the
        // catch-all to pass one and break the other.
        AgentEvent::ReasoningText { .. } => return None,
        // The final bounded answer is rendered through AgentOutput's existing
        // truncated marker; this status has no standalone pipe text.
        AgentEvent::AnsweringBudgetStopped { .. } => return None,
        // The token counts one provider call reported. This is data the JSON/NDJSON
        // adapter carries whole; the text adapter renders it to nothing (the
        // interactive surfaces for it are the per-turn token line and `/usage`,
        // and a pipe's reader has the answer above it). It must not fall through
        // to the catch-all below, which would print `unrecognized agent event`
        // under a correct answer — the same regression that has shipped for
        // contentless variants here three times.
        AgentEvent::Usage { call, usage } => TerminalEvent::Usage { call, usage },
        AgentEvent::FailedAttemptUsage { usage } => TerminalEvent::FailedAttemptUsage { usage },
        AgentEvent::ProviderRecovery {
            phase,
            reason,
            attempt,
            limit,
        } => TerminalEvent::ProviderRecovery {
            phase,
            reason,
            attempt,
            limit,
        },
        AgentEvent::AnswerDesignated { sql } => TerminalEvent::AnswerDesignated { sql },
        AgentEvent::ConsensusDecided {
            sql,
            attempts,
            voted,
            votes,
            margin,
            tied,
            probe_broke_tie,
        } => TerminalEvent::ConsensusDecided {
            sql,
            attempts,
            voted,
            votes,
            margin,
            tied,
            probe_broke_tie,
        },
        AgentEvent::Complete => TerminalEvent::Complete,
        // The turn's provider stream failed mid-answer and the loop is
        // retrying it. Carried under its own type tag so a machine consumer
        // can replace the text it accumulated for this turn; the text adapter
        // prints a notice (see `TerminalEvent::TurnReset`).
        AgentEvent::TurnReset => TerminalEvent::TurnReset,
        // The model stopped to ask the user one focused question instead of
        // assuming (B3c). The turn ends here; the question block prints and
        // the process exits 6 (the paused class) for `saya ask`.
        AgentEvent::ClarificationNeeded { question, options } => {
            TerminalEvent::ClarificationNeeded { question, options }
        }
        // AgentEvent is #[non_exhaustive]; a future variant this renderer does not
        // yet understand must not silently terminate the stream (Complete) — surface
        // it as an unimplemented event instead.
        _ => TerminalEvent::NotImplemented {
            feature: "unrecognized agent event".into(),
        },
    })
}
