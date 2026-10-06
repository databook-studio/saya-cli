//! ADR 0009's bounds table against this crate's `pub` constants. Each needle
//! is the ADR's `value | \`CONSTANT\`` cells with the value derived from the
//! constant itself, so a doc number or a code value drifting from the other
//! fails here.

use std::path::PathBuf;

fn adr_text() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/adr-0009-native-coding-and-continuity.md"),
    )
    .expect("docs/adr-0009-native-coding-and-continuity.md exists at the repo root")
}

fn assert_adr_states(needle: String) {
    let adr = adr_text();
    assert!(
        adr.contains(&needle),
        "the ADR bounds table must state `{needle}`; either the doc number \
         or this crate's constant moved"
    );
}

#[test]
fn adr_states_the_continuation_bounds() {
    use saya_types::{
        MAX_CHECK_FACTS, MAX_CONTINUATION_BYTES, MAX_FILE_FACTS, MAX_NARRATIVE_BYTES,
        MAX_USER_NOTES_BYTES,
    };
    assert_adr_states(format!(
        "{} B | `MAX_CONTINUATION_BYTES`",
        MAX_CONTINUATION_BYTES
    ));
    assert_adr_states(format!("{} B | `MAX_NARRATIVE_BYTES`", MAX_NARRATIVE_BYTES));
    assert_adr_states(format!(
        "{} B | `MAX_USER_NOTES_BYTES`",
        MAX_USER_NOTES_BYTES
    ));
    assert_adr_states(format!("{} | `MAX_FILE_FACTS`", MAX_FILE_FACTS));
    assert_adr_states(format!("{} | `MAX_CHECK_FACTS`", MAX_CHECK_FACTS));
}

#[test]
fn adr_states_the_preference_bounds() {
    use saya_types::{
        MAX_PREFERENCE_RECALL_BYTES, MAX_PREFERENCE_TEXT_BYTES, MAX_PROJECT_PREFERENCES,
    };
    assert_adr_states(format!(
        "{} | `MAX_PROJECT_PREFERENCES`",
        MAX_PROJECT_PREFERENCES
    ));
    assert_adr_states(format!(
        "{} B | `MAX_PREFERENCE_TEXT_BYTES`",
        MAX_PREFERENCE_TEXT_BYTES
    ));
    assert_adr_states(format!(
        "{} B | `MAX_PREFERENCE_RECALL_BYTES`",
        MAX_PREFERENCE_RECALL_BYTES
    ));
}

#[test]
fn adr_states_the_checkpoint_bounds() {
    use saya_types::{
        MAX_CHECKPOINT_FILE_BYTES, MAX_CHECKPOINT_SESSION_BYTES, MAX_CHECKPOINTS_PER_SESSION,
    };
    assert_adr_states(format!(
        "{} B | `MAX_CHECKPOINT_FILE_BYTES`",
        MAX_CHECKPOINT_FILE_BYTES
    ));
    assert_adr_states(format!(
        "{} B | `MAX_CHECKPOINT_SESSION_BYTES`",
        MAX_CHECKPOINT_SESSION_BYTES
    ));
    assert_adr_states(format!(
        "{} | `MAX_CHECKPOINTS_PER_SESSION`",
        MAX_CHECKPOINTS_PER_SESSION
    ));
}
