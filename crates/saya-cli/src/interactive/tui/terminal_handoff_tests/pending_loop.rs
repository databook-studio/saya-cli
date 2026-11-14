/// `/setup` typed while busy waits like every other command: the busy gate
/// holds the queued line and the handoff never starts.
#[test]
fn setup_typed_while_busy_waits_like_every_other_command() {
    let mut fx = PendingFixture::build("busy-gate");
    fx.app.pending = Some("/setup".to_string());
    // A running compaction makes the app busy, exactly like a query or an
    // agent stream would.
    let (_tx, rx) = std::sync::mpsc::channel::<CompactOutcome>();
    fx.app.compact_task = Some(rx);
    let mut recorder = Recorder::ok();
    fx.drain(&mut recorder).expect("the busy tick runs");
    assert_eq!(
        fx.app.pending.as_deref(),
        Some("/setup"),
        "the busy gate holds the line"
    );
    assert!(
        recorder.log.entries().is_empty(),
        "the handoff never ran while busy: {:?}",
        recorder.log.entries()
    );
}

/// A handoff whose release fails returns the io error — the loop stops —
/// and the best-effort repair still reacquired the half-released terminal.
/// The release failing means the wizard never ran (no stdin read), so this
/// exercises the real `tick_pending` path with a fake.
#[test]
fn a_failed_handoff_returns_the_error_and_stops_the_loop() {
    let mut fx = PendingFixture::build("failed-handoff");
    fx.app.pending = Some("/setup".to_string());
    let mut recorder = Recorder::failing_release();
    let error = fx
        .drain(&mut recorder)
        .expect_err("a failed handoff stops the loop");
    assert!(error.to_string().contains("release failed"), "{error}");
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "the half-released terminal gets its best-effort repair"
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .all(|block| !block.text.contains("Guided setup")),
        "nothing renders on a lost handoff"
    );
}
