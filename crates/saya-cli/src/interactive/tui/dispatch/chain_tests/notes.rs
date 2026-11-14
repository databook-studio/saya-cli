/// `/notes` through the real dispatch with no continuation record: the
/// show is a System block carrying the no-notes words and the remedy.
#[test]
fn notes_dispatch_shows_the_no_notes_words_without_a_record() {
    let mut fx = ChainFixture::build("notes-show");
    assert!(
        matches!(dispatch_line("/notes", &mut fx), Dispatch::Handled),
        "the show is handled inline"
    );
    let shown = last_block(&fx.app.transcript, BlockKind::System).expect("/notes pushes a block");
    assert!(
        shown.contains("No session notes") && shown.contains("/notes set <text>"),
        "the show names the remedy: {shown}"
    );
}

/// `/notes set` without a continuation record: the Error block carries the
/// /compact-first refusal — never a silent no-op, never a created record.
#[test]
fn notes_set_dispatch_refuses_without_a_continuation_record() {
    let mut fx = ChainFixture::build("notes-no-record");
    assert!(
        matches!(
            dispatch_line("/notes set keep these", &mut fx),
            Dispatch::Handled
        ),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("run /compact first"),
        "the refusal names the /compact remedy: {error}"
    );
    assert!(
        fx.state.continuation.is_none(),
        "no record was created by the refusal"
    );
}

/// `/notes set` over a seeded `Current` record: the System block says the
/// notes are stored, the state carries them, and the record still
/// validates.
#[test]
fn notes_set_dispatch_stores_into_a_seeded_record() {
    use saya_types::{CONTINUATION_VERSION, SessionContinuationV1, SourceBoundary};
    let mut fx = ChainFixture::build("notes-seeded");
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 1,
            source_sha256: "c".repeat(64),
            workspace_root: None,
            profile: None,
        },
        narrative: None,
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    };
    fx.state.continuation = Some(saya_store::StoredContinuation::Current(record));
    assert!(
        matches!(
            dispatch_line("/notes set keep these", &mut fx),
            Dispatch::Handled
        ),
        "the store is inline"
    );
    let stored = last_block(&fx.app.transcript, BlockKind::System).expect("the success is said");
    assert!(
        stored.contains("Session notes stored."),
        "the success is the stored words: {stored}"
    );
    let Some(saya_store::StoredContinuation::Current(updated)) = &fx.state.continuation else {
        panic!("the record is still typed: {:?}", fx.state.continuation);
    };
    assert_eq!(updated.user_notes.as_deref(), Some("keep these"));
    updated
        .validate()
        .expect("the updated record still validates");
}
