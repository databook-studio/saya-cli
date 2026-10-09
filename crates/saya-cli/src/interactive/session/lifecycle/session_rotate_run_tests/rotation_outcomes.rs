/// A completed rotation: the parent saved and unchanged, the successor
/// published with the store's lineage link, the state swapped to the tail
/// and carried summary — and nothing inherited: grants empty, recovery
/// off, the old lock released, the new lock held.
#[test]
fn a_completed_rotation_publishes_and_swaps() {
    let mut fx = Fixture::build("success");
    fx.session.policy().grants().grant("sql:demo");
    fx.session.recovery().set_enabled(true);
    let tail = fx.state.turns[fx.state.compacted_turns..].to_vec();
    let parent_record = fx.state.redacted();

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Rotated { old, new } = outcome else {
        panic!("a compacted session rotates: {outcome:?}");
    };
    assert_eq!(old, OLD);
    assert_eq!(new, SUCCESSOR);

    assert_eq!(fx.state.id, SUCCESSOR, "the successor is live");
    assert_eq!(fx.state.turns, tail, "the verbatim tail rides across");
    assert_eq!(fx.state.compacted_turns, 0);
    assert_eq!(
        fx.state.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried narrative is the live summary"
    );
    assert_eq!(
        fx.state.parent,
        Some(ParentLink {
            id: OLD.into(),
            rotation_id: SUCCESSOR.into()
        })
    );
    assert!(
        fx.session.policy().grants().is_empty(),
        "a successor inherits no grants"
    );
    assert!(
        !fx.session.recovery().is_enabled(),
        "a successor starts with recovery off"
    );

    let parent = block_on(fx.store.load(OLD))
        .expect("the parent loads")
        .expect("the parent record exists");
    assert_eq!(parent.id, parent_record.id);
    assert_eq!(parent.turns, parent_record.turns, "the parent is unchanged");
    assert_eq!(parent.parent, None, "the parent carries no link");

    let successor = block_on(fx.store.load(SUCCESSOR))
        .expect("the successor loads")
        .expect("the successor record exists");
    assert_eq!(
        successor.parent,
        Some(ParentLink {
            id: OLD.into(),
            rotation_id: SUCCESSOR.into()
        }),
        "the store wrote the lineage link"
    );

    assert!(
        fx.reacquire(OLD).is_ok(),
        "the old session's lock is released"
    );
    let error = fx
        .reacquire(SUCCESSOR)
        .err()
        .expect("the successor's lock is held");
    assert!(error.contains("already running"), "{error}");
}

/// An uncompacted session refuses with the sentence, and nothing rotated:
/// the session stands and no successor record exists.
#[test]
fn an_uncompacted_session_refuses_and_nothing_rotates() {
    let mut fx = Fixture::build("uncompacted");
    let mut plain = SessionState::new(OLD, None, "m");
    for index in 0..6 {
        plain.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    fx.state = plain;

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Refused(message) = outcome else {
        panic!("an uncompacted session refuses: {outcome:?}");
    };
    assert_eq!(
        message,
        "Nothing to carry over yet: run /compact first so the session has a summary."
    );
    assert_eq!(fx.state.id, OLD, "the session stands");
    assert!(
        block_on(fx.store.load(SUCCESSOR))
            .expect("the store loads")
            .is_none(),
        "no successor record was created"
    );
}

/// A record that would exceed the 4 MiB cap refuses with the
/// compact-further sentence: no successor exists, the session stands.
#[test]
fn an_over_cap_session_refuses_with_the_compact_further_sentence() {
    let mut fx = Fixture::build("over-cap");
    // The not-yet-summarised tail made huge: the continuation boundary
    // covers only the compacted prefix, so the record still validates.
    let huge = "x".repeat(5 << 20);
    fx.state.turns[4].user = huge.clone();

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Refused(message) = outcome else {
        panic!("an over-cap session refuses: {outcome:?}");
    };
    assert_eq!(
        message,
        "The successor would still exceed the 4 MiB session limit; compact further before rotating."
    );
    assert_eq!(fx.state.id, OLD, "the session stands");
    assert!(
        block_on(fx.store.load(SUCCESSOR))
            .expect("the store loads")
            .is_none(),
        "no successor record exists"
    );
    assert_eq!(fx.state.turns[4].user, huge, "the session is unchanged");
}
