/// A paused run's NDJSON stream is one JSON object per line, every line —
/// lifecycle lines tagged `"type"`, interleaved with the episode's events in
/// the `TerminalEvent` envelope tagged `"event"`. The pause lands as a
/// `paused` line naming its reason, and the terminal settle message is a JSON
/// error line on stderr, never stray text.
#[test]
fn an_ndjson_run_stream_parses_as_one_json_object_per_line_when_paused() {
    let env = test_root("ndjson-pause");
    let (slow, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the workspace"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("step answer"),
            delay_ms: 2_500,
        },
    ]);
    let output = saya(
        &env,
        &[
            "--format",
            "ndjson",
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "--budget",
            "wall-clock=1",
            "survey the data quality",
        ],
        &slow,
    );
    assert_eq!(
        output.status.code(),
        Some(6),
        "the budget trips and the run pauses; stderr: {}",
        stderr(&output)
    );

    // Every line, both streams, is one JSON object — nothing else is ever
    // printed on the wire.
    let out = json_lines(&stdout(&output), "stdout");
    let _err = json_lines(&stderr(&output), "stderr");

    // The lifecycle rode the wire in journal order.
    let tags = type_tags(&out);
    for expected in ["run_started", "plan_approved", "step_started", "paused"] {
        assert!(
            tags.contains(&expected),
            "the stream must carry a {expected} line: {out:?}"
        );
    }
    // The episode's events are present under today's envelope — the two tags
    // share one stream by design, and the dual-tag stream stays one
    // JSON object per line either way.
    assert!(
        out.iter()
            .any(|value| value.get("event").and_then(|tag| tag.as_str()).is_some()),
        "the episode events ride the TerminalEvent envelope: {out:?}"
    );
    let paused = out
        .iter()
        .find(|value| value.get("type").and_then(|tag| tag.as_str()) == Some("paused"))
        .expect("a paused line");
    assert_eq!(
        paused.get("reason").and_then(|reason| reason.as_str()),
        Some("wall_clock_exceeded"),
        "the pause names its cause: {paused:?}"
    );

    // The resume speaks the same wire. Its whole-run clock includes time
    // already spent before the pause, so an expired ceiling stops before a
    // new provider request.
    let id = {
        let listing = saya(&env, &["run", "list"], &slow);
        newest_run_id(&listing)
    };
    let (fast, fast_ready) = mock(vec![Scripted {
        body: sse("remaining step answer"),
        delay_ms: 0,
    }]);
    let resumed = saya(&env, &["--format", "ndjson", "run", "resume", &id], &fast);
    assert_eq!(
        resumed.status.code(),
        Some(6),
        "the whole-run wall-clock budget includes pause time; stderr: {}",
        stderr(&resumed)
    );
    assert!(
        !fast_ready.load(Ordering::SeqCst),
        "an expired carried deadline must stop before another provider request"
    );
    let resumed_lines = json_lines(&stdout(&resumed), "resumed stdout");
    json_lines(&stderr(&resumed), "resumed stderr");
    let resumed_tags = type_tags(&resumed_lines);
    assert!(
        resumed_tags.contains(&"paused") && !resumed_tags.contains(&"completed"),
        "the same spent whole-run deadline remains paused on resume: {resumed_lines:?}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A failing run's stream is one JSON object per line too: the bounded
/// retries surface as step_failed lines, the pause is said out loud, and the
/// failure message is a JSON error line on stderr — never stray text that a
/// line-oriented consumer would choke on.
#[test]
fn an_ndjson_run_stream_stays_line_oriented_when_the_run_fails() {
    let env = test_root("ndjson-fail");
    // The plan binds, then every episode call fails: the mock has spent its
    // script, so each further connection is accepted and dropped — a provider
    // failure, never a hang. The engine retries the step bounded, then pauses.
    let (failing, _ready) = mock(vec![Scripted {
        body: plan_body(&["a step that cannot succeed"]),
        delay_ms: 0,
    }]);
    let output = saya(
        &env,
        &[
            "--format",
            "ndjson",
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "a goal whose steps all fail",
        ],
        &failing,
    );
    assert_eq!(
        output.status.code(),
        Some(6),
        "the spent retries pause the run; stderr: {}",
        stderr(&output)
    );
    let out = json_lines(&stdout(&output), "stdout");
    let _err = json_lines(&stderr(&output), "stderr");
    let tags = type_tags(&out);
    assert!(
        tags.iter().filter(|tag| **tag == "step_failed").count() >= 3,
        "the bounded retries are on the wire: {out:?}"
    );
    let paused = out
        .iter()
        .find(|value| value.get("type").and_then(|tag| tag.as_str()) == Some("paused"))
        .expect("a paused line");
    assert_eq!(
        paused.get("reason").and_then(|reason| reason.as_str()),
        Some("step_failed_after_retry"),
        "the pause names the spent retries: {paused:?}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
