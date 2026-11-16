//! Editing a record and its timestamps: an edit keeps identity and bumps
//! only `updated`, never rebinds the worktree, refuses invalid text, and
//! timestamps must be ordered and non-negative.

use super::*;

// ---------------------------------------------------------------------------
// Edits keep identity; only text and updated change
// ---------------------------------------------------------------------------

#[test]
fn edited_keeps_identity_and_bumps_updated() {
    let base = record();
    let edited = base.edited("prefer compact tables", CREATED + 5).unwrap();
    assert_eq!(edited.id, base.id);
    assert_eq!(edited.worktree, base.worktree);
    assert_eq!(edited.source, PreferenceSource::UserExplicit);
    assert_eq!(edited.created_unix_ms, CREATED);
    assert_eq!(edited.updated_unix_ms, CREATED + 5);
    assert_eq!(edited.text, "prefer compact tables");
    // `now` before `created` is clamped, never reordered.
    let clamped = base.edited("prefer compact tables", CREATED - 100).unwrap();
    assert_eq!(clamped.updated_unix_ms, CREATED);
}

#[test]
fn edited_refuses_invalid_text_and_never_rebinds_the_worktree() {
    let base = record();
    assert_eq!(
        base.edited("", CREATED + 1).unwrap_err(),
        PreferenceError::TextEmpty
    );
    assert_eq!(
        base.edited(&"a".repeat(MAX_PREFERENCE_TEXT_BYTES + 1), CREATED + 1)
            .unwrap_err(),
        PreferenceError::TextOversize(MAX_PREFERENCE_TEXT_BYTES + 1)
    );
    // The edit takes no worktree parameter, so rebinding is unrepresentable;
    // the result carries the original worktree however the edit is made.
    assert_ne!(other_worktree(), base.worktree);
    let edited = base.edited("same worktree only", CREATED + 1).unwrap();
    assert_eq!(edited.worktree, base.worktree);
}

// ---------------------------------------------------------------------------
// Timestamps
// ---------------------------------------------------------------------------

#[test]
fn timestamps_must_be_ordered_and_non_negative() {
    let mut late = record();
    late.updated_unix_ms = CREATED - 1;
    assert_eq!(
        late.validate().unwrap_err(),
        PreferenceError::TimestampOrder
    );
    let mut negative = record();
    negative.created_unix_ms = -1;
    negative.updated_unix_ms = -1;
    assert_eq!(
        negative.validate().unwrap_err(),
        PreferenceError::NegativeTimestamp
    );
}
