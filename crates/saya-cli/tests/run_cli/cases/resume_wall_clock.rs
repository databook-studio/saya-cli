/// A wall-clock pause carries across invocations: the slow first episode
/// consumes the whole ceiling, and a later resume stays paused without a
/// fresh provider call.
#[test]
fn a_run_paused_by_its_wall_clock_does_not_recharge_on_resume() {
    let env = test_root("paused-budget");
    let (slow, _slow_ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the workspace", "finish the survey"]),
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
        "a run paused by its budget exits 6; stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("paused"),
        "the pause must be said out loud: {}",
        stderr(&output)
    );
    let listing = saya(&env, &["run", "list"], &slow);
    assert_eq!(listing.status.code(), Some(0));
    let id = newest_run_id(&listing);
    assert!(!id.is_empty(), "the paused run must be listed: {id}");
    assert!(
        stdout(&listing).contains("paused"),
        "the listing must show the paused status: {}",
        stdout(&listing)
    );
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    let journal_lines = json_lines(&journal, "run journal");
    let journal_tags = type_tags(&journal_lines);
    let approved = journal_tags
        .iter()
        .position(|tag| *tag == "plan_approved")
        .unwrap();
    let clock = journal_tags
        .iter()
        .position(|tag| *tag == "wall_clock_observed")
        .unwrap();
    let step = journal_tags
        .iter()
        .position(|tag| *tag == "step_started")
        .unwrap();
    assert!(
        approved < clock && clock < step,
        "clock must be durable after approval and before step work: {journal_tags:?}"
    );

    let (fast, fast_called) = mock(vec![Scripted {
        body: sse("remaining step answer"),
        delay_ms: 0,
    }]);
    let resumed = saya(&env, &["run", "resume", &id], &fast);
    assert_eq!(
        resumed.status.code(),
        Some(6),
        "the exhausted whole-run clock keeps resume paused; stderr: {}",
        stderr(&resumed)
    );
    assert!(
        !fast_called.load(Ordering::SeqCst),
        "an exhausted resume must not invoke the provider"
    );
    let (fast_again, fast_again_called) = mock(vec![Scripted {
        body: sse("another remaining step answer"),
        delay_ms: 0,
    }]);
    let resumed_again = saya(&env, &["run", "resume", &id], &fast_again);
    assert_eq!(
        resumed_again.status.code(),
        Some(6),
        "the same exhausted whole-run clock keeps a second resume paused; stderr: {}",
        stderr(&resumed_again)
    );
    assert!(
        !fast_again_called.load(Ordering::SeqCst),
        "a second exhausted resume must not invoke the provider"
    );
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    let clock_marks: Vec<(u64, u64)> = json_lines(&journal, "run journal")
        .iter()
        .filter(|event| event["type"] == "wall_clock_observed")
        .map(|event| {
            (
                event["origin_unix_ms"].as_u64().unwrap(),
                event["high_water_unix_ms"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(
        clock_marks.len() >= 3,
        "both resumes must persist an observation: {clock_marks:?}"
    );
    assert!(
        clock_marks
            .iter()
            .all(|(origin, _)| *origin == clock_marks[0].0)
    );
    assert!(clock_marks.windows(2).all(|pair| pair[1].1 >= pair[0].1));
    assert!(journal.contains("wall_clock_exceeded"));
    let _ = fs::remove_dir_all(&env.root);
}
