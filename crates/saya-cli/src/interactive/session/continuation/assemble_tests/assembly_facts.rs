/// A compaction that completes saves a typed continuation: the deterministic
/// facts carry the observed edit with the injected digest, the clean summary
/// becomes the generated narrative, the source boundary is the persisted turn
/// count, and the transcript (`messages`, `turns`) is unchanged.
#[test]
fn compaction_saves_a_continuation_with_facts_and_a_clean_narrative() {
    let mut state = turns(5);
    observe_edit(&mut state, "src/a.rs");
    let before_messages = state.messages.clone();
    let before_turns = state.turns.clone();
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of)
        .expect("a valid summary applies");
    let record = current(&state);
    assert_eq!(
        record.source.through_turn, 5,
        "the boundary is the turn count"
    );
    assert_eq!(
        record.files.len(),
        1,
        "the observed edit is a fact: {record:?}"
    );
    assert_eq!(record.files[0].path, "src/a.rs");
    assert_eq!(
        record.files[0].sha256.as_deref(),
        Some(digest64().as_str()),
        "the digest comes from the injected resolver"
    );
    let narrative = record
        .narrative
        .as_ref()
        .expect("a clean narrative survives");
    assert_eq!(narrative.text, SUMMARY);
    assert_eq!(narrative.source, NarrativeSource::Summarizer);
    record.validate().expect("the record validates");
    assert_eq!(
        state.messages, before_messages,
        "the transcript is unchanged"
    );
    assert_eq!(state.turns, before_turns, "the turns are unchanged");
}

/// A summary the redactor would rewrite is OMITTED from the continuation —
/// the facts still save. The live working-memory summary is unchanged.
#[test]
fn a_sensitive_narrative_is_omitted_but_facts_survive() {
    let summary = "setup used password=hunter2 and finished the edit.";
    assert_ne!(
        redact(summary),
        summary,
        "the test needs a sensitive summary"
    );
    let mut state = turns(5);
    observe_edit(&mut state, "src/a.rs");
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, summary, &digest_of).expect("applies");
    let record = current(&state);
    assert!(
        record.narrative.is_none(),
        "a sensitive narrative must be omitted: {:?}",
        record.narrative
    );
    assert_eq!(record.files.len(), 1, "the facts survive the omission");
    record.validate().expect("the fact-only record validates");
    assert_eq!(
        state.compaction_summary.as_deref(),
        Some(summary),
        "the live working memory keeps the summary as before"
    );
}

/// The model's prose mints no facts: a summary claiming edits and passing
/// tests records no file or check the tools did not observe.
#[test]
fn a_forged_summary_adds_no_facts() {
    let mut state = turns(5);
    let plan = plan_for(&state);
    apply_with_continuation(
        &mut state,
        &plan,
        "I edited secret.rs and all tests passed.",
        &digest_of,
    )
    .expect("applies");
    let record = current(&state);
    assert!(
        record.files.is_empty() && record.checks.is_empty(),
        "prose minted facts: {record:?}"
    );
}

/// User-authored notes ride across a new compaction, preserved apart from the
/// generated narrative — the new record carries them and a fresh boundary.
#[test]
fn existing_user_notes_are_preserved_across_a_new_compaction() {
    let mut state = turns(5);
    state.continuation = Some(StoredContinuation::Current(seeded_current(Some(
        "keep these notes",
    ))));
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    let record = current(&state);
    assert_eq!(
        record.user_notes.as_deref(),
        Some("keep these notes"),
        "the notes ride the new record"
    );
    assert_ne!(
        record.source.source_sha256,
        "b".repeat(64),
        "the boundary is the new compaction's, not the seeded one"
    );
    assert!(
        record.narrative.is_some(),
        "the narrative is still generated"
    );
}

/// An `Unsupported` continuation is a newer binary's data: a compaction
/// applies its working-memory change but never replaces the record.
#[test]
fn an_unsupported_continuation_is_never_replaced() {
    let mut state = turns(5);
    let unsupported = StoredContinuation::Unsupported(serde_json::json!({"version": 2}));
    state.continuation = Some(unsupported.clone());
    let plan = plan_for(&state);
    apply_with_continuation(&mut state, &plan, SUMMARY, &digest_of).expect("applies");
    assert_eq!(
        state.continuation,
        Some(unsupported),
        "the newer binary's continuation must survive the compaction"
    );
    assert!(
        state.compaction_summary.is_some(),
        "the working memory still compacts"
    );
}
