use super::*;

// ---------------------------------------------------------------------------
// The narrative bound: exactly MAX_NARRATIVE_BYTES, never empty, only
// newline and tab as control characters
// ---------------------------------------------------------------------------

#[test]
fn the_narrative_admits_exactly_max_bytes_and_refuses_one_more() {
    let mut record = minimal();
    record.narrative = narrative(&"a".repeat(MAX_NARRATIVE_BYTES));
    record
        .validate()
        .expect("exactly 8 KiB of narrative is inside the bound");

    record.narrative = narrative(&"a".repeat(MAX_NARRATIVE_BYTES + 1));
    assert_eq!(
        record.validate(),
        Err(ContinuationError::NarrativeOversize(
            MAX_NARRATIVE_BYTES + 1
        ))
    );
}

#[test]
fn the_narrative_must_not_be_empty() {
    let mut record = minimal();
    record.narrative = narrative("");
    assert_eq!(record.validate(), Err(ContinuationError::NarrativeEmpty));
}

#[test]
fn narrative_and_notes_allow_newline_and_tab_only() {
    let mut record = minimal();
    record.narrative = narrative("line one\nline two\tindented");
    record.user_notes = Some("note\nmore\ttabs".to_string());
    record
        .validate()
        .expect("newline and tab are allowed prose control");

    record.narrative = narrative("bell\u{0007}");
    assert_eq!(
        record.validate(),
        Err(ContinuationError::NarrativeControlCharacter)
    );
    record.narrative = narrative("carriage\rreturn");
    assert_eq!(
        record.validate(),
        Err(ContinuationError::NarrativeControlCharacter)
    );
    record.narrative = None;
    record.user_notes = Some("carriage\rreturn".to_string());
    assert_eq!(
        record.validate(),
        Err(ContinuationError::UserNotesControlCharacter)
    );
    record.user_notes = Some("nul\u{0}byte".to_string());
    assert_eq!(
        record.validate(),
        Err(ContinuationError::UserNotesControlCharacter)
    );
}

// ---------------------------------------------------------------------------
// The user-notes bound: exactly MAX_USER_NOTES_BYTES, never empty
// ---------------------------------------------------------------------------

#[test]
fn the_user_notes_admit_exactly_max_bytes_and_refuse_one_more() {
    let mut record = minimal();
    record.user_notes = Some("a".repeat(MAX_USER_NOTES_BYTES));
    record
        .validate()
        .expect("exactly 8 KiB of notes is inside the bound");

    record.user_notes = Some("a".repeat(MAX_USER_NOTES_BYTES + 1));
    assert_eq!(
        record.validate(),
        Err(ContinuationError::UserNotesOversize(
            MAX_USER_NOTES_BYTES + 1
        ))
    );
}

#[test]
fn user_notes_must_not_be_empty() {
    let mut record = minimal();
    record.user_notes = Some(String::new());
    assert_eq!(record.validate(), Err(ContinuationError::UserNotesEmpty));
}

// ---------------------------------------------------------------------------
// The check label bound: bounded like a task title
// ---------------------------------------------------------------------------

#[test]
fn the_check_label_admits_exactly_max_chars_and_refuses_one_more() {
    let mut record = minimal();
    record.checks = vec![check(&"a".repeat(MAX_CHECK_LABEL_CHARS))];
    record
        .validate()
        .expect("a 200-character label is inside the bound");

    record.checks = vec![check(&"a".repeat(MAX_CHECK_LABEL_CHARS + 1))];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
}

#[test]
fn empty_and_control_character_labels_are_refused() {
    let mut record = minimal();
    record.checks = vec![check("")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
    record.checks = vec![check("   ")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
    record.checks = vec![check("label\u{0007}bell")];
    assert_eq!(record.validate(), Err(ContinuationError::InvalidCheckLabel));
}
