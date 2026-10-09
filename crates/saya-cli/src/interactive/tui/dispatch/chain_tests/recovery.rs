/// `/recovery` through the real dispatch: the status names the off state,
/// `/recovery on` says the retention text and flips the session's own
/// flag, and `/recovery list` renders the (empty) listing.
#[test]
fn recovery_status_on_and_list_dispatch_through_the_chain() {
    let mut fx = ChainFixture::build("recovery-chain");
    assert!(!fx.session.recovery().is_enabled(), "opt-in: off at start");
    assert!(
        matches!(dispatch_line("/recovery", &mut fx), Dispatch::Handled),
        "the status is inline"
    );
    let status = last_block(&fx.app.transcript, BlockKind::System).expect("the status is said");
    assert!(
        status.contains("Recovery is off"),
        "the status names the flag: {status}"
    );

    assert!(
        matches!(dispatch_line("/recovery on", &mut fx), Dispatch::Handled),
        "the toggle is inline"
    );
    let on = last_block(&fx.app.transcript, BlockKind::System).expect("the answer is said");
    assert!(
        on.starts_with("Recovery is on."),
        "the retention text is said: {on}"
    );
    assert!(
        on.contains("never commands, downloads, scratch tables or databases"),
        "the retention text says what it does not cover: {on}"
    );
    assert!(
        fx.session.recovery().is_enabled(),
        "the flag flipped on the session's own handle"
    );

    assert!(
        matches!(dispatch_line("/recovery list", &mut fx), Dispatch::Handled),
        "the listing is inline"
    );
    let list = last_block(&fx.app.transcript, BlockKind::System).expect("the list is said");
    assert!(
        list.contains("No recorded edits"),
        "an empty session says so: {list}"
    );
}

/// `/recovery restore` of an id nothing recorded lands as an Error block
/// through the real dispatch — the unresolved id never reaches the store.
#[test]
fn recovery_restore_of_an_unknown_id_errors_through_the_chain() {
    let mut fx = ChainFixture::build("recovery-restore-unknown");
    assert!(
        matches!(
            dispatch_line("/recovery restore ck-000000", &mut fx),
            Dispatch::Handled
        ),
        "the restore is inline"
    );
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is said");
    assert!(
        error.contains("no recorded edit"),
        "the unknown id is named: {error}"
    );
}

/// The busy rule the TUI holds for restores: while a request is running
/// (here a compaction, the same busy signal `App::is_busy()` covers for a
/// stream, query or replay) a queued `/recovery restore` waits — the
/// restore dispatches only once the request has finished, so it can never
/// overlap a turn that may be writing the same file. Proven through the
/// real loop tick, the same machinery the typed `/resume` and `/compact`
/// pass through.
#[test]
fn a_queued_restore_waits_for_a_running_request() {
    let mut fx = ChainFixture::build("recovery-busy");
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.pending = Some("/recovery restore ck-000000".to_string());
    let blocks_before = fx.app.transcript.blocks().len();

    drain_through_tick_pending(&mut fx);
    assert_eq!(
        fx.app.transcript.blocks().len(),
        blocks_before,
        "while busy, the tick dispatches nothing"
    );
    assert!(
        fx.app.pending.is_some(),
        "the restore stays queued while the request runs"
    );

    // The request finished: the queued restore now dispatches, and the
    // unknown id is refused exactly as an immediate dispatch would.
    fx.app.compact_task = None;
    drain_through_tick_pending(&mut fx);
    assert!(fx.app.pending.is_none(), "the queued restore was consumed");
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is said");
    assert!(
        error.contains("no recorded edit"),
        "the restore ran only once idle: {error}"
    );
}
