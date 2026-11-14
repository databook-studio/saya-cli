/// Happy path: a long history compacts; the result is summary + verbatim
/// tail; the transcript (`messages`) is untouched.
#[test]
fn a_long_history_compacts_to_summary_plus_verbatim_tail() {
    let mut state = turns(5);
    let before_messages = state.messages.clone();
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    assert_eq!(plan.compacted_turns, 3);
    let summary = "older turns: q0 q1 q2 discussed orders.";
    apply(&mut state, &plan, summary).expect("valid summary applies");
    let replayed = state.provider_history();
    assert!(
        replayed[0].content.contains(summary),
        "the summary replays first: {:?}",
        replayed[0].content
    );
    assert!(
        replayed
            .iter()
            .any(|message| message.content == "q3" && message.role == "user"),
        "the verbatim tail survives: {replayed:?}"
    );
    assert!(
        replayed
            .iter()
            .any(|message| message.content == "a4" && message.role == "assistant"),
        "the newest answer survives: {replayed:?}"
    );
    assert_eq!(
        state.messages, before_messages,
        "the transcript is untouched by compaction"
    );
}

/// Pinned tokens survive: a summarised assistant text carrying a file size
/// and digest pins those exact strings, and a summary that preserves them
/// applies while the tail stays verbatim.
#[test]
fn pinned_size_and_digest_survive_compaction() {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..5 {
        state.record_turn(
            format!("write chunk {index}"),
            format!(
                "appended notes.md\npath: notes.md\nsize: {}\ndigest: deadbeef{index:04x}",
                100 + index
            ),
            false,
            Vec::new(),
        );
    }
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    assert!(
        plan.pinned.iter().any(|token| token.contains("size: 100")),
        "the oldest size is pinned: {:?}",
        plan.pinned
    );
    assert!(
        plan.pinned
            .iter()
            .any(|token| token.contains("digest: deadbeef")),
        "the oldest digest is pinned: {:?}",
        plan.pinned
    );
    assert!(
        plan.pinned
            .iter()
            .any(|token| token.contains("path: notes.md")),
        "the path naming the anchored file is pinned: {:?}",
        plan.pinned
    );
    let mut summary = String::from("older writes landed; ");
    for token in &plan.pinned {
        summary.push_str(token);
        summary.push_str("; ");
    }
    apply(&mut state, &plan, &summary).expect("a preserving summary applies");
    let replayed = state.provider_history();
    assert!(
        replayed[0].content.contains("size: 100"),
        "the size survives: {:?}",
        replayed[0].content
    );
    assert!(
        replayed[0].content.contains("digest: deadbeef0000"),
        "the digest survives: {:?}",
        replayed[0].content
    );
    // The newest tool-result group was never summarised: it replays verbatim.
    assert!(
        replayed
            .iter()
            .any(|message| message.content.contains("size: 104")),
        "the newest size replays verbatim, not via the summary: {replayed:?}"
    );
}

/// A summary that drops a pinned token is rejected and the conversation is
/// unchanged — including no stored summary.
#[test]
fn a_summary_that_drops_a_pinned_token_is_rejected_unchanged() {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..5 {
        state.record_turn(
            format!("write chunk {index}"),
            format!(
                "appended\nsize: {}\ndigest: deadbeef{index:04x}",
                100 + index
            ),
            false,
            Vec::new(),
        );
    }
    let before = state.provider_history();
    let history = before.clone();
    let plan = plan(&state.turns, &history).expect("long history plans");
    assert!(!plan.pinned.is_empty(), "the test needs a pinned token");
    let error = apply(&mut state, &plan, "older writes landed, details omitted.")
        .expect_err("a pin-dropping summary must be rejected");
    assert!(
        error.contains("pinned"),
        "the rejection names the pinned loss: {error}"
    );
    assert!(
        state.compaction_summary.is_none(),
        "no summary is stored on rejection"
    );
    assert_eq!(
        state.provider_history(),
        before,
        "the conversation is unchanged after rejection"
    );
    // Direct validation agrees: the unit and the apply path share one rule.
    assert!(validate("details omitted.", &plan.pinned).is_err());
}
