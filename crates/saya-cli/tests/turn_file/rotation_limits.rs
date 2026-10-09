use super::*;

/// `/rotate` through the real headless binary: a `--turn-file` run starts a
/// fresh session, so the rotation always refuses with the /compact-first
/// remedy — exit 5, the streamed error names it, no provider round runs,
/// and stdout stays valid NDJSON. The success path is covered by the
/// `session_rotate_run` unit tests against a real store and runtime.
#[test]
fn turn_file_rotate_refuses_a_fresh_session_and_exits_five() {
    let env = test_root("rotate-slash");
    let (address, bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/rotate\n").unwrap();
    let mut args = base_args(&env);
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(5),
        "the refusal exits 5; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("run /compact first"),
        "the refusal names the /compact remedy: {stderr}"
    );
    assert!(
        bodies.lock().unwrap().is_empty(),
        "a slash command runs no provider round"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().filter(|line| !line.is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|error| panic!("stdout line is not NDJSON: {line:?}: {error}"));
    }
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A resumed session whose stored record sits at the warning level: the one
/// turn's save lands over 3 MiB, the near-limit notice rides stdout once as
/// a `result` event before the `turn_result` event, and the turn still
/// completes with exit 0. The record is pre-written as a minimal legacy
/// shape the store accepts (all fields `#[serde(default)]`), one turn whose
/// assistant filler puts the stored file between 3 MiB (warn) and 4 MiB
/// (the save ceiling).
#[test]
fn a_resumed_record_at_the_warning_level_says_the_near_limit_notice_once() {
    let env = test_root("capacity-notice");
    let (address, _bodies) = mock("unused");
    let id = "capacity-notice";
    let filler = "x".repeat(3_350_000);
    let record = format!(r#"{{"id":"{id}","turns":[{{"user":"q","assistant":"{filler}"}}]}}"#);
    std::fs::write(env.sessions.join(format!("{id}.json")), record).unwrap();
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/help\n").unwrap();
    let mut args = base_args(&env);
    args.push("--resume".into());
    args.push(id.into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "a warn-level record still saves; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    let notices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("of its 4 MiB limit"))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        notices.len(),
        1,
        "the near-limit notice is said exactly once: {stdout}"
    );
    let turn_result = lines
        .iter()
        .position(|line| line.contains("\"event\":\"turn_result\""))
        .expect("the scripted run ends with its turn_result");
    assert!(
        notices[0] < turn_result,
        "the notice comes before the turn_result event: {stdout}"
    );
    let notice_line: serde_json::Value =
        serde_json::from_str(lines[notices[0]]).unwrap_or_else(|error| {
            panic!("the notice line is NDJSON: {error}: {}", lines[notices[0]])
        });
    assert_eq!(
        notice_line["event"], "result",
        "the notice rides the result event: {notice_line}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A real turn that pushes the record over the 4 MiB ceiling: the turn's
/// save is refused before publishing, the friendly refusal — never the raw
/// store error — rides the stream, and the scripted run exits 5 (Errored).
/// (A resumed record over the cap cannot even load — the store bounds reads
/// too — so the refusal is driven by the turn's own verbatim bytes on top of
/// a resumed record just under the ceiling.)
#[test]
fn a_turn_that_pushes_the_record_over_the_cap_refuses_with_the_friendly_sentence() {
    let env = test_root("capacity-limit");
    let (address, _bodies) = mock("done");
    let id = "capacity-limit";
    let filler = "x".repeat(3_950_000);
    let record = format!(r#"{{"id":"{id}","turns":[{{"user":"q","assistant":"{filler}"}}]}}"#);
    let record_path = env.sessions.join(format!("{id}.json"));
    std::fs::write(&record_path, record.clone()).unwrap();
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "x".repeat(500_000)).unwrap();
    let mut args = base_args(&env);
    args.push("--resume".into());
    args.push(id.into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(5),
        "the refused save is an errored run; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        all.contains("This session's record has reached its 4 MiB limit"),
        "the friendly refusal is said: {all}"
    );
    assert!(
        all.contains("/resume capacity-limit restores the last saved state"),
        "the refusal names the resume path: {all}"
    );
    assert!(
        !all.contains("value exceeds a store limit"),
        "the raw store error never shows: {all}"
    );
    assert_eq!(
        std::fs::read_to_string(&record_path).expect("the record reads"),
        record,
        "the last good record on disk is byte-identical: the refusal published nothing"
    );
    // The one terminal result tells the truth: the refused save ends the run
    // errored, so the last stdout line reports failed/5 — never the
    // completed/0 the turn's own settlement would render.
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    let results: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.contains("\"event\":\"turn_result\""))
        .collect();
    assert_eq!(
        results.len(),
        1,
        "exactly one turn_result ends the run: {stdout}"
    );
    assert_eq!(
        lines.last().copied(),
        Some(results[0]),
        "the turn_result is the last stdout line: {stdout}"
    );
    assert_eq!(
        results[0],
        "{\"event\":\"turn_result\",\"status\":\"failed\",\"exit_code\":5,\"tools_completed\":0,\"tools_unfinished\":[]}",
        "the refused save reports failed/5: {stdout}"
    );
    // The friendly sentence is said exactly once: the per-turn save's
    // refusal carries it, and the end-of-run save's identical repeat is
    // never repeated.
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let limit_errors = stderr
        .lines()
        .filter(|line| line.contains("\"event\":\"error\"") && line.contains("4 MiB limit"))
        .count();
    assert_eq!(
        limit_errors, 1,
        "exactly one error event carries the limit sentence: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
