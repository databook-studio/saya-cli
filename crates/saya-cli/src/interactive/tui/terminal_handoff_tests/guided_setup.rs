/// The cancellation path: EOF at the first prompt cancels the wizard with
/// exit 0 and "No files were changed.", and the handoff still restored the
/// terminal exactly once each way.
#[test]
fn guided_setup_eof_cancels_and_restores_once() {
    let dir = temp_dir("eof");
    let mut recorder = Recorder::ok();
    let (probes, calls) = recording_probes();
    let mut options = flow_options(&dir, probes);
    let mut input = Cursor::new("");
    let mut output = Vec::new();
    let result = run_guided_setup_with(&mut recorder, &mut options, &mut input, &mut output);
    let handoff = result.expect("EOF cancels the wizard, never the handoff");
    assert_eq!(
        handoff,
        SetupHandoff::Finished(0),
        "the wizard's own cancel is exit 0"
    );
    let out = String::from_utf8(output).unwrap();
    assert!(
        out.contains("No files were changed."),
        "the cancel line: {out}"
    );
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "exactly one release and one reacquire"
    );
    let recorded = calls.lock().unwrap().clone();
    assert!(recorded.is_empty(), "no probe ever ran: {recorded:?}");
    assert!(
        !dir.join("connections.toml").exists(),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(dir);
}

/// The wizard-error path: the simplest forced `run_with` error (the
/// flow_tests pattern — an unreadable recovery marker) surfaces as
/// `Failed(..)`, and the terminal is still reacquired.
#[test]
fn guided_setup_wizard_error_still_reacquires() {
    let dir = temp_dir("wizard-error");
    fs::write(dir.join(MARKER_FILE), b"{ interrupted, but corrupt").unwrap();
    let mut recorder = Recorder::ok();
    let (probes, calls) = recording_probes();
    let mut options = flow_options(&dir, probes);
    let mut input = Cursor::new(FULL_SCRIPT);
    let mut output = Vec::new();
    let result = run_guided_setup_with(&mut recorder, &mut options, &mut input, &mut output);
    let handoff = result.expect("a wizard error is a Failed handoff, not an io error");
    match handoff {
        SetupHandoff::Failed(message) => {
            assert!(
                message.contains(MARKER_FILE),
                "the failure names the marker: {message}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        recorder.log.entries(),
        vec!["release", "reacquire"],
        "the wizard's error still returns the terminal"
    );
    let recorded = calls.lock().unwrap().clone();
    assert!(recorded.is_empty(), "no probe ever ran: {recorded:?}");
    assert!(
        !dir.join("connections.toml").exists(),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(dir);
}

/// `run_guided_setup` locks the real stdin/stdout, so calling it inside a
/// test would block on the developer's terminal; U06 calls it in production.
/// This pins its existence and signature — the honest alternative to a
/// dead-code allow.
#[test]
fn run_guided_setup_is_the_real_streams_entry_point() {
    let entry: fn(&mut Recorder, &mut FlowOptions) -> io::Result<SetupHandoff> =
        run_guided_setup::<Recorder>;
    let _ = entry;
}
