use super::*;

#[test]
fn continuation_prose_is_redacted_before_it_reaches_disk() {
    let root = temp_root("continuation-redaction");
    let store = FsSessionStore::new(&root);
    let record = safe_continuation(
        "summary password=veil-marrow postgres://u:zeal@host/db",
        "password=coral-drift",
        "gate password=latch-ember",
    );

    block_on(store.save(RedactedSession {
        id: "redacted-continuation".into(),
        continuation: Some(StoredContinuation::Current(record)),
        ..Default::default()
    }))
    .unwrap();

    let saved = std::fs::read_to_string(root.join("redacted-continuation.json")).unwrap();
    for secret in ["veil-marrow", "coral-drift", "latch-ember", "u:zeal@"] {
        assert!(!saved.contains(secret), "leaked {secret}");
    }

    let loaded = block_on(store.load("redacted-continuation"))
        .unwrap()
        .unwrap();
    let Some(StoredContinuation::Current(round_tripped)) = loaded.continuation else {
        panic!(
            "the redacted continuation must stay typed: {:?}",
            loaded.continuation
        );
    };
    let narrative = round_tripped.narrative.as_ref().unwrap();
    assert!(narrative.text.contains("[redacted]"));
    assert!(!narrative.text.contains("veil-marrow"));
    assert!(!narrative.text.contains("zeal@"));
    assert!(
        round_tripped
            .user_notes
            .as_ref()
            .unwrap()
            .contains("[redacted]")
    );
    assert!(
        !round_tripped
            .user_notes
            .as_ref()
            .unwrap()
            .contains("coral-drift")
    );
    assert!(round_tripped.checks[0].label.contains("[redacted]"));
    assert!(!round_tripped.checks[0].label.contains("latch-ember"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_over_cap_continuation_is_refused_and_the_old_bytes_survive() {
    let root = temp_root("continuation-over-cap");
    let store = FsSessionStore::new(&root);
    block_on(store.save(RedactedSession {
        id: "capped".into(),
        messages: vec![RedactedMessage {
            role: "user".into(),
            content: "old".into(),
        }],
        ..Default::default()
    }))
    .unwrap();
    let path = root.join("capped.json");
    let before = std::fs::read(&path).unwrap();

    let mut record = safe_continuation("fine", "fine", "cargo test");
    record.narrative = Some(GeneratedNarrative {
        text: "x".repeat(9 * 1024),
        source: NarrativeSource::Deterministic,
    });
    let error = block_on(store.save(RedactedSession {
        id: "capped".into(),
        continuation: Some(StoredContinuation::Current(record)),
        ..Default::default()
    }))
    .expect_err("an over-cap continuation must be refused");
    assert_eq!(error, StoreError::LimitExceeded);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a refused save must leave the previous bytes untouched"
    );
    let _ = std::fs::remove_dir_all(root);
}
