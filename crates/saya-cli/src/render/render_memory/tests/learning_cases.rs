// --- Spec packet-54: the KnowledgeLearningSkipped shaper. ---

/// The two reasons render the exact spec strings and read differently — a
/// timeout and a failure must not collapse (packet-54 decision 3).
#[test]
fn the_two_skip_reasons_render_the_spec_strings_and_differ() {
    use saya_agent::LearningSkipReason;
    let timed_out = learning_skipped_text(LearningSkipReason::TimedOut);
    let failed = learning_skipped_text(LearningSkipReason::Failed);
    assert_eq!(
        timed_out,
        "memory not recorded · extraction timed out; this turn was not learned from\n",
    );
    assert_eq!(
        failed,
        "memory not recorded · extraction failed; this turn was not learned from\n",
    );
    assert_ne!(timed_out, failed);
    // Both name what happened plainly, without claiming memory is broken.
    assert!(timed_out.contains("memory not recorded"));
    assert!(failed.contains("memory not recorded"));
    assert!(timed_out.contains("not learned from"));
    assert!(failed.contains("not learned from"));
}

// --- Learning breaker: the KnowledgeLearningDisabled shaper. ---

/// The shaper's exact text (owner decision 3), naming the model and the
/// miss count and stating that recall and `/remember` are unaffected.
#[test]
fn learning_disabled_text_is_the_exact_spec_line() {
    let text = learning_disabled_text("glm-5.2", 2);
    assert_eq!(
        text,
        "memory: learning disabled for this session — extraction with glm-5.2 was cut off \
         or stalled 2 times in a row. Recall still works; /remember still stores a rule.\n"
    );
}
