/// `/rotate` on an uncompacted session: the Error block carries the
/// /compact-first refusal, the session stands, nothing published.
#[test]
fn rotate_dispatch_refuses_an_uncompacted_session() {
    let mut fx = ChainFixture::build("rotate-refusal");
    assert!(
        matches!(dispatch_line("/rotate", &mut fx), Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("run /compact first"),
        "the refusal names the /compact remedy: {error}"
    );
    assert_eq!(fx.state.id, "chain-test", "nothing rotated");
}

/// `/rotate` on a compacted session: the System block carries the exact
/// success sentence, the state is the successor, and the published record
/// links back to the parent.
#[test]
fn rotate_dispatch_rotates_a_compacted_session() {
    let mut fx = ChainFixture::build("rotate-ok");
    compact_fixture_state(&mut fx.state);
    assert!(
        matches!(dispatch_line("/rotate", &mut fx), Dispatch::Handled),
        "the rotation is inline"
    );
    let new = fx.state.id.clone();
    assert_ne!(new, "chain-test", "the state is the successor");
    let message = last_block(&fx.app.transcript, BlockKind::System).expect("the success is said");
    assert_eq!(
        message,
        crate::interactive::session::lifecycle::rotate_run::rotated_message("chain-test", &new),
        "the success sentence is the shared one"
    );
    let successor = crate::interactive::session::lifecycle::resume::block_on(fx.store.load(&new))
        .expect("the successor loads")
        .expect("the successor record exists");
    assert_eq!(
        successor.parent,
        Some(saya_store::ParentLink {
            id: "chain-test".into(),
            rotation_id: new
        }),
        "the store wrote the lineage link"
    );
}

/// A `/rotate` submitted while a request is running WAITS: the queue gate
/// holds the line (the same handling a typed `/resume` relies on — the
/// dispatch chain only ever runs when the app is idle), nothing rotates,
/// and once the request ends the queued line drains through the real
/// dispatch and the rotation runs its refusal.
#[test]
fn a_rotate_submitted_while_busy_waits_for_the_running_request() {
    let mut fx = ChainFixture::build("rotate-busy");
    let before = fx.state.id.clone();
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.input.set_text("/rotate");
    fx.app.submit();
    assert_eq!(
        fx.app.pending.as_deref(),
        Some("/rotate"),
        "the busy submit queued the line"
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Queued")),
        "the queue notice says the line waits: {:?}",
        fx.app
            .transcript
            .blocks()
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(fx.state.id, before, "nothing rotated while busy");

    // The request ends: the queued line drains through the real dispatch.
    fx.app.compact_task = None;
    drain_through_tick_pending(&mut fx);
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the queued rotation ran");
    assert!(
        error.contains("run /compact first"),
        "the uncompacted refusal, once idle: {error}"
    );
    assert_eq!(fx.state.id, before, "the refusal changed nothing");
    wait_for_background_save(&fx.root.0.join("session-store"));
}
