/// Shapes the text line for one [`AgentEvent::KnowledgeLearningSkipped`]
/// event, for any adapter that prints it. Trails the answer —
/// the runtime emits it after the loop — so it lands below the assistant text,
/// where "and I did not learn from this turn" belongs.
///
/// Wording mirrors the recall precedent (`memory off · recall disabled`,
/// `memory skipped · not permitted…`): the line says plainly that nothing was
/// recorded and why, and that *this turn* was not learned from — not that
/// memory is broken. A timeout and a failure read differently so a user (and a
/// review) can tell them apart without re-deriving the outcome. The two
/// strings are fixed by the spec; this shaper exists so the headless path and
/// the TUI cannot disagree, the same arrangement `knowledge_supplied_text`
/// uses. `LearningSkipReason` is `#[non_exhaustive]`: a future variant this
/// shaper does not know about renders a generic line rather than panic.
pub(crate) fn learning_skipped_text(reason: LearningSkipReason) -> String {
    match reason {
        LearningSkipReason::TimedOut => {
            "memory not recorded · extraction timed out; this turn was not learned from\n".into()
        }
        LearningSkipReason::Failed => {
            "memory not recorded · extraction failed; this turn was not learned from\n".into()
        }
        // A future skip reason this shaper does not yet know about: name it as a
        // skip without guessing the cause. The turn still completes (rendering
        // never fails it).
        _ => "memory not recorded · this turn was not learned from\n".into(),
    }
}

/// Shapes the text line for one [`AgentEvent::KnowledgeLearningDisabled`]
/// event — the extraction circuit breaker's once-per-session notice. Trails
/// the answer, immediately after that turn's own [`learning_skipped_text`]
/// line, so the two read together: "this turn wasn't learned from, and
/// here's why nothing further will be either." The wording is fixed by the
/// spec: it names the model and the miss count, and it says plainly that
/// recall and `/remember` still work — the breaker stops one call, not the
/// feature.
pub(crate) fn learning_disabled_text(model: &str, misses: u32) -> String {
    format!(
        "memory: learning disabled for this session — extraction with {model} was cut off or \
         stalled {misses} times in a row. Recall still works; /remember still stores a rule.\n"
    )
}
