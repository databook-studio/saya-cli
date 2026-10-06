//! Resume-time restoration of a saved session's continuation: the round
//! trip through the real store, every rejection reason, and the guarantees
//! around them — a rejected record is never dropped, and the runtime privacy
//! gate still governs what a restored summary feeds.

use super::{RejectReason, RestoreOutcome, restore_continuation};
use crate::interactive::session_compact::{apply_with_continuation, plan as compact_plan};
use crate::interactive::session_continuation::record::source_digest;
use crate::interactive::session_resume::{SessionDefaults, block_on, state_from_redacted};
use crate::interactive::session_state::SessionState;
use saya_store::{FsSessionStore, SessionStore, StoredContinuation};
use saya_types::{
    CONTINUATION_VERSION, GeneratedNarrative, NarrativeSource, SessionContinuationV1,
    SourceBoundary, redact,
};

const SUMMARY: &str = "older turns discussed orders";

fn turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

fn defaults() -> SessionDefaults {
    SessionDefaults {
        provider: "ollama".into(),
        model: "m".into(),
        allow_data_sharing: true,
        approval_mode: "ask".into(),
    }
}

/// Compacts `state` through the production path: the plan plus
/// `apply_with_continuation` with the test digest resolver (no workspace
/// files to resolve).
fn compact(state: &mut SessionState, summary: &str) {
    let history = state.provider_history();
    let plan = compact_plan(&state.turns, &history).expect("a long history plans");
    apply_with_continuation(state, &plan, summary, &|_| None).expect("the summary applies");
}

fn store_root(label: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-restore-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create the store root");
    root
}

/// Saves `state` through the real store and resumes it through the one path
/// every resume goes through: `state_from_redacted`.
fn save_and_resume(root: &std::path::Path, state: &SessionState) -> SessionState {
    let store = FsSessionStore::new(root);
    block_on(store.save(state.redacted())).expect("the record saves");
    let loaded = block_on(store.load("s1"))
        .expect("loads")
        .expect("the record is there");
    state_from_redacted(loaded, &defaults())
}

/// A rejected continuation leaves the session exactly as any other resume:
/// no summary, no compacted turns, every persisted turn replayed.
fn assert_resumed_unchanged(state: &SessionState, turn_count: usize) {
    assert!(state.compaction_summary.is_none(), "no summary is restored");
    assert_eq!(state.compacted_turns, 0, "no turns sit behind a summary");
    let replayed = state.provider_request_history();
    assert_eq!(
        replayed.len(),
        turn_count * 2,
        "every persisted turn replays: {replayed:?}"
    );
    assert_eq!(
        replayed[replayed.len() - 1].content,
        format!("a{}", turn_count - 1)
    );
}

/// A compacted session resumes with its summary and the verbatim tail: the
/// next request replays the last two turns beside the summary instead of the
/// whole transcript.
#[test]
fn a_compacted_session_resumes_with_summary_and_verbatim_tail() {
    let root = store_root("summary");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let resumed = save_and_resume(&root, &state);
    assert_eq!(
        resumed.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the narrative is restored as the live summary"
    );
    assert_eq!(
        resumed.compacted_turns, 4,
        "four turns sit behind the summary"
    );
    assert_eq!(
        resumed.compaction_summary_text().as_deref(),
        Some(SUMMARY),
        "the permissive classification keeps the gate open"
    );
    let replayed = resumed.provider_request_history();
    assert_eq!(replayed.len(), 4, "only the tail replays: {replayed:?}");
    assert_eq!(replayed[0].role, "user");
    assert_eq!(replayed[0].content, "q4");
    assert_eq!(replayed[1].role, "assistant");
    assert_eq!(replayed[1].content, "a4");
    assert_eq!(replayed[2].content, "q5");
    assert_eq!(replayed[3].content, "a5");
    let mut again = resumed.clone();
    assert!(
        matches!(
            restore_continuation(&mut again),
            RestoreOutcome::Restored { compacted_turns: 4 }
        ),
        "the restored outcome names the boundary"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Turns appended after the record was written do not invalidate the
/// restore: the prefix digest still matches, the boundary stays put, and the
/// request history carries the tail plus the new turns.
#[test]
fn turns_appended_after_compaction_keep_the_restore_valid() {
    let root = store_root("appended");
    let store = FsSessionStore::new(&root);
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    block_on(store.save(state.redacted())).expect("the compaction record saves");
    state.record_turn("q6", "a6", false, Vec::new());
    state.record_turn("q7", "a7", false, Vec::new());
    block_on(store.save(state.redacted())).expect("the appended record saves");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.compacted_turns, 4,
        "the boundary keeps the old prefix"
    );
    assert_eq!(resumed.compaction_summary.as_deref(), Some(SUMMARY));
    let replayed = resumed.provider_request_history();
    assert_eq!(
        replayed.len(),
        8,
        "the tail and the new turns replay: {replayed:?}"
    );
    for (index, expected) in ["q4", "a4", "q5", "a5", "q6", "a6", "q7", "a7"]
        .iter()
        .enumerate()
    {
        assert_eq!(replayed[index].content, *expected, "{replayed:?}");
    }
    let _ = std::fs::remove_dir_all(root);
}

/// An edited persisted turn breaks the prefix digest: the continuation is
/// rejected and the session resumes with every turn replayed.
#[test]
fn a_changed_stored_turn_rejects_the_continuation_as_turns_changed() {
    let root = store_root("turns-changed");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.turns[0].user = "edited q0".into();
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::TurnsChanged)
    ));
    assert_resumed_unchanged(&resumed, 6);
    let _ = std::fs::remove_dir_all(root);
}

/// A changed workspace root means the record no longer describes this
/// session: rejected, and the resumed session keeps the record's root.
#[test]
fn a_changed_workspace_rejects_the_continuation_as_workspace_changed() {
    let root = store_root("workspace-changed");
    let mut state = turns(6);
    state.workspace_root = Some("/projects/saya".into());
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.workspace_root = Some("/elsewhere".into());
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::WorkspaceChanged)
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.workspace_root.as_deref(),
        Some("/elsewhere"),
        "the resumed session keeps the record's root"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A changed profile means the record no longer describes this session.
#[test]
fn a_changed_profile_rejects_the_continuation_as_profile_changed() {
    let root = store_root("profile-changed");
    let mut state = turns(6);
    state.profile = Some("analytics".into());
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.profile = Some("other".into());
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::ProfileChanged)
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.profile.as_deref(),
        Some("other"),
        "the resumed session keeps the record's profile"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A boundary past the persisted turn count cannot name this transcript.
#[test]
fn a_truncated_transcript_rejects_the_continuation_as_boundary_out_of_range() {
    let root = store_root("boundary");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    loaded.turns.truncate(3);
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::BoundaryOutOfRange)
    ));
    assert_resumed_unchanged(&resumed, 3);
    let _ = std::fs::remove_dir_all(root);
}

/// A narrative the redactor would rewrite is refused at restore time —
/// defense in depth beside the save-time omission.
#[test]
fn an_unsafe_narrative_is_rejected_as_narrative_unsafe() {
    let mut state = turns(6);
    let text = "password=hunter2";
    assert_ne!(
        redact(text),
        text,
        "the test needs a redactor-rewritten narrative"
    );
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 6,
            source_sha256: source_digest(&state.turns),
            workspace_root: state.workspace_root.clone(),
            profile: state.profile.clone(),
        },
        narrative: Some(GeneratedNarrative {
            text: text.into(),
            source: NarrativeSource::Summarizer,
        }),
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    };
    record
        .validate()
        .expect("the hand-built record passes the contract");
    state.continuation = Some(StoredContinuation::Current(record));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::NarrativeUnsafe)
    ));
    assert_resumed_unchanged(&state, 6);
}

/// A newer binary's continuation is never interpreted: it is rejected and
/// the value stays untouched.
#[test]
fn an_unsupported_continuation_is_rejected_and_untouched() {
    let mut state = turns(6);
    let raw = serde_json::json!({"version": 2, "future": true});
    state.continuation = Some(StoredContinuation::Unsupported(raw.clone()));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::Unsupported)
    ));
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Unsupported(raw)),
        "the value is untouched"
    );
    assert_resumed_unchanged(&state, 6);
}

/// A session without a continuation has nothing to restore.
#[test]
fn a_session_without_a_continuation_resumes_without_one() {
    let mut state = turns(6);
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::NoContinuation
    ));
    assert_resumed_unchanged(&state, 6);
}

/// A summary the redactor rewrites was omitted from the saved record; the
/// resume restores no summary but keeps the fact record on the state.
#[test]
fn a_sensitive_narrative_omitted_at_save_resumes_without_a_summary() {
    let root = store_root("no-narrative");
    let sensitive = "setup used password=hunter2 and finished the edit.";
    assert_ne!(redact(sensitive), sensitive);
    let mut state = turns(6);
    compact(&mut state, sensitive);
    let record = match &state.continuation {
        Some(StoredContinuation::Current(record)) => record.clone(),
        other => panic!("the compaction saved a typed record: {other:?}"),
    };
    assert!(
        record.narrative.is_none(),
        "the sensitive narrative was omitted at save"
    );
    let resumed = save_and_resume(&root, &state);
    let mut again = resumed.clone();
    assert!(matches!(
        restore_continuation(&mut again),
        RestoreOutcome::NoNarrative
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.continuation,
        Some(StoredContinuation::Current(record)),
        "the fact record stays on the resumed state"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A rejected continuation is never dropped or rewritten: the state keeps
/// exactly what it loaded, `redacted()` hands it back, and the next save
/// persists it.
#[test]
fn a_rejected_continuation_is_never_dropped() {
    let root = store_root("kept");
    let mut state = turns(6);
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let mut loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let expected = loaded.continuation.clone();
    loaded.turns[1].assistant = "edited a1".into();
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::Rejected(RejectReason::TurnsChanged)
    ));
    assert_eq!(
        resumed.continuation, expected,
        "the loaded record is untouched"
    );
    assert_eq!(
        resumed.redacted().continuation,
        expected,
        "redacted() hands it back to the store"
    );
    block_on(store.save(resumed.redacted())).expect("the next save keeps the rejected record");
    let reloaded = block_on(store.load("s1")).expect("loads").expect("present");
    assert_eq!(
        reloaded.continuation, expected,
        "the rejected record survives the save"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// The runtime privacy gate still holds after a restore: the summary is on
/// the session, but a cloud provider without data sharing never receives it.
#[test]
fn a_restored_summary_stays_behind_the_privacy_gate() {
    let root = store_root("privacy");
    let mut state = turns(5);
    state.provider = "openai".into();
    state.record_turn("q5", "a5", true, Vec::new());
    compact(&mut state, SUMMARY);
    let store = FsSessionStore::new(&root);
    block_on(store.save(state.redacted())).expect("saves");
    let loaded = block_on(store.load("s1")).expect("loads").expect("present");
    let resumed = state_from_redacted(
        loaded,
        &SessionDefaults {
            provider: "openai".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    );
    assert_eq!(
        resumed.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the summary itself is restored; the gate is a request-time rule"
    );
    assert!(
        resumed.compaction_summary_text().is_none(),
        "a database-derived turn under a cloud provider keeps the gate shut"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A boundary-zero record is the rotation-carried case: the narrative
/// summarises nothing in THIS session's turns, so it restores as a summary
/// over an empty compacted region and the record stays untouched.
fn carried_record(
    digest: String,
    workspace: Option<String>,
    profile: Option<String>,
) -> SessionContinuationV1 {
    SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 0,
            source_sha256: digest,
            workspace_root: workspace,
            profile,
        },
        narrative: Some(GeneratedNarrative {
            text: SUMMARY.into(),
            source: NarrativeSource::Summarizer,
        }),
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    }
}

/// A boundary-zero record restores as a carried summary: the digest over the
/// empty prefix matches, so the narrative rides in with `compacted_turns`
/// zero — it summarises nothing in this session's own turns.
#[test]
fn a_boundary_zero_record_restores_as_a_carried_summary() {
    let mut state = turns(2);
    let record = carried_record(source_digest(&[]), None, None);
    record
        .validate()
        .expect("the hand-built carried record passes the contract");
    state.continuation = Some(StoredContinuation::Current(record.clone()));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Restored { compacted_turns: 0 }
    ));
    assert_eq!(
        state.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried narrative is restored as the live summary"
    );
    assert_eq!(
        state.compacted_turns, 0,
        "nothing in this session is behind it"
    );
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Current(record)),
        "the record stays untouched"
    );
}

/// A boundary-zero record whose digest is not the empty-prefix digest does
/// not describe a carried summary: rejected, record untouched, plain resume.
#[test]
fn a_carried_record_with_a_wrong_empty_digest_rejects_as_turns_changed() {
    let mut state = turns(2);
    let record = carried_record("f".repeat(64), None, None);
    state.continuation = Some(StoredContinuation::Current(record.clone()));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::TurnsChanged)
    ));
    assert_resumed_unchanged(&state, 2);
    assert_eq!(
        state.continuation,
        Some(StoredContinuation::Current(record)),
        "the record stays untouched"
    );
}

/// A boundary-zero record carries the workspace it was written under: a
/// different root means it no longer describes this session.
#[test]
fn a_carried_record_with_a_changed_workspace_rejects_as_workspace_changed() {
    let mut state = turns(2);
    state.workspace_root = Some("/projects/saya".into());
    let record = carried_record(source_digest(&[]), Some("/elsewhere".into()), None);
    state.continuation = Some(StoredContinuation::Current(record));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::WorkspaceChanged)
    ));
    assert_resumed_unchanged(&state, 2);
}

/// A boundary-zero record carries the profile it was written under: a
/// different profile means it no longer describes this session.
#[test]
fn a_carried_record_with_a_changed_profile_rejects_as_profile_changed() {
    let mut state = turns(2);
    state.profile = Some("analytics".into());
    let record = carried_record(source_digest(&[]), None, Some("other".into()));
    state.continuation = Some(StoredContinuation::Current(record));
    assert!(matches!(
        restore_continuation(&mut state),
        RestoreOutcome::Rejected(RejectReason::ProfileChanged)
    ));
    assert_resumed_unchanged(&state, 2);
}

/// The old boundary rule is unchanged for real boundaries: 1 and 2 sit at or
/// below the kept tail, so they stay out of range even when their digests
/// match their prefixes exactly.
#[test]
fn boundaries_1_and_2_still_reject_as_boundary_out_of_range() {
    for through in [1usize, 2] {
        let mut state = turns(6);
        let record = SessionContinuationV1 {
            version: CONTINUATION_VERSION,
            source: SourceBoundary {
                through_turn: through as u64,
                source_sha256: source_digest(&state.turns[..through]),
                workspace_root: None,
                profile: None,
            },
            narrative: Some(GeneratedNarrative {
                text: SUMMARY.into(),
                source: NarrativeSource::Summarizer,
            }),
            user_notes: None,
            files: Vec::new(),
            checks: Vec::new(),
        };
        record
            .validate()
            .expect("the hand-built record passes the contract");
        state.continuation = Some(StoredContinuation::Current(record));
        assert!(
            matches!(
                restore_continuation(&mut state),
                RestoreOutcome::Rejected(RejectReason::BoundaryOutOfRange)
            ),
            "through={through} stays out of range"
        );
        assert_resumed_unchanged(&state, 6);
    }
}

/// A session written before continuations existed resumes byte-for-byte as
/// before: the raw fixture carries no `continuation` key, and
/// deserialization — not struct construction — proves the old record loads.
#[test]
fn an_old_session_without_a_continuation_field_resumes_unchanged() {
    let root = store_root("legacy");
    std::fs::write(
        root.join("old.json"),
        r#"{"version":2,"id":"old","profile_names":[],"turns":[{"user":"q0","assistant":"a0"},{"user":"q1","assistant":"a1"},{"user":"q2","assistant":"a2"},{"user":"q3","assistant":"a3"},{"user":"q4","assistant":"a4"},{"user":"q5","assistant":"a5"}],"messages":[]}"#,
    )
    .expect("writes the legacy fixture");
    let store = FsSessionStore::new(&root);
    let loaded = block_on(store.load("old"))
        .expect("loads")
        .expect("present");
    let mut resumed = state_from_redacted(loaded, &defaults());
    assert!(matches!(
        restore_continuation(&mut resumed),
        RestoreOutcome::NoContinuation
    ));
    assert_resumed_unchanged(&resumed, 6);
    assert_eq!(
        resumed.provider_request_history()[0].content,
        "q0",
        "the old record replays from its first turn, byte-for-byte"
    );
    let _ = std::fs::remove_dir_all(root);
}
