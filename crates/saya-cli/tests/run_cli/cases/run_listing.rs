/// `saya run list` and `saya run show <id>` render a run that exists; `show`
/// on an unknown id fails cleanly rather than panicking.
#[test]
fn list_and_show_render_a_run_and_an_unknown_id_fails_cleanly() {
    let env = test_root("list-show");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the schema"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("survey complete"),
            delay_ms: 0,
        },
    ]);
    let started = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "survey the schema",
        ],
        &address,
    );
    assert_eq!(
        started.status.code(),
        Some(0),
        "the one-step run must complete; stderr: {}",
        stderr(&started)
    );
    let listing = saya(&env, &["run", "list"], &address);
    assert_eq!(listing.status.code(), Some(0));
    let id = newest_run_id(&listing);
    assert!(
        !id.is_empty(),
        "the completed run must be listed: {}",
        stdout(&listing)
    );
    let show = saya(&env, &["run", "show", &id], &address);
    assert_eq!(show.status.code(), Some(0), "stderr: {}", stderr(&show));
    let shown = stdout(&show);
    assert!(
        shown.contains("completed"),
        "the show must report the status: {shown}"
    );
    assert!(
        shown.contains("survey the schema"),
        "the show must report the run's goal: {shown}"
    );
    let log = saya(&env, &["run", "log", &id], &address);
    assert_eq!(log.status.code(), Some(0), "stderr: {}", stderr(&log));
    let logged = stdout(&log);
    // Text mode renders the shaper's own lines: what happened, in journal
    // order. The wire bytes stay under `--format ndjson`, asserted below.
    assert!(
        logged.contains("run started") && logged.contains("plan approved"),
        "the log must carry the journal events: {logged}"
    );
    let wire = saya(&env, &["--format", "ndjson", "run", "log", &id], &address);
    assert_eq!(wire.status.code(), Some(0), "stderr: {}", stderr(&wire));
    let wired = stdout(&wire);
    assert!(
        wired.contains("run_started") && wired.contains("plan_approved"),
        "the NDJSON framing carries the journal's own lines: {wired}"
    );
    // An unknown id fails cleanly: non-zero, a message that names the miss,
    // and never a panic (101).
    let unknown = saya(&env, &["run", "show", "r-nope"], &address);
    assert_ne!(
        unknown.status.code(),
        Some(101),
        "an unknown run id must not panic; stderr: {}",
        stderr(&unknown)
    );
    assert_ne!(
        unknown.status.code(),
        Some(0),
        "an unknown run id must fail"
    );
    assert!(
        stderr(&unknown).contains("no run with id"),
        "the failure must say why: {}",
        stderr(&unknown)
    );
    let _ = fs::remove_dir_all(&env.root);
}

// ---------------------------------------------------------------------------
// The run wire (NDJSON): lifecycle `RunEvent` lines tagged "type", interleaved
// with the episode events in today's `TerminalEvent` envelope tagged "event".
// The stream is the journal itself — every line is one JSON object, and
// consumers that read `event` keys see no new episode events.
// ---------------------------------------------------------------------------
