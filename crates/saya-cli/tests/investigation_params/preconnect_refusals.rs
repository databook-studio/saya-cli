/// A missing required parameter refuses with the required names and types
/// before the run touches a profile: an unknown `--connection` would be the
/// first failure of any later step, so the parameter refusal winning over it
/// proves the check ran before connection work.
#[tokio::test]
async fn missing_parameter_fails_before_connect() {
    let h = harness_with_database("missing-required");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string:required"],
    );

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "no-such-profile",
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "the missing required parameter refuses: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("missing required parameter") && stderr.contains("label (string)"),
        "the refusal lists the required names with types: {stderr}"
    );
    assert!(
        stderr.contains("--param"),
        "the refusal names the remedy: {stderr}"
    );
    assert!(
        !stderr.contains("unknown profile"),
        "the refusal is not the profile error — it fired first: {stderr}"
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}

/// An unknown `--param` name refuses before connecting, naming the declared
/// parameters — never a value.
#[tokio::test]
async fn unknown_parameter_name_fails_before_connect() {
    let h = harness_with_database("unknown-param");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "no-such-profile",
        "--param",
        "region=east",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("region") && stderr.contains("declared"),
        "the refusal names the unknown parameter and the declared ones: {stderr}"
    );
    assert!(
        !stderr.contains("unknown profile"),
        "the refusal is not the profile error — it fired first: {stderr}"
    );
    let _ = fs::remove_dir_all(h.root);
}

/// A value that does not parse as its declared type refuses before
/// connecting; the error text never echoes the value.
#[tokio::test]
async fn value_parse_failure_refuses_before_connect() {
    let h = harness_with_database("bad-value");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "Since day",
        "SELECT id FROM events WHERE id > :since",
        &["since:date"],
    );

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "no-such-profile",
        "--param",
        "since=not-a-date",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("since") && stderr.contains("date"),
        "the refusal names the parameter and its declared type: {stderr}"
    );
    assert!(
        !stderr.contains("not-a-date"),
        "a value must never leak into the error text: {stderr}"
    );
    assert!(
        !stderr.contains("unknown profile"),
        "the refusal is not the profile error — it fired first: {stderr}"
    );
    let _ = fs::remove_dir_all(h.root);
}
