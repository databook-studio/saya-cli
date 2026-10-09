#[tokio::test]
async fn parameter_values_never_persist() {
    const SENTINEL: &str = "HVNS3NT1NEL42";
    let h = harness_with_database("no-persist");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );
    let report = h.root.join("report.md");

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--param",
        &format!("label={SENTINEL}"),
        "--report",
        report.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    let stderr = h.stderr(&output);
    assert!(
        !stdout.contains(SENTINEL) && !stderr.contains(SENTINEL),
        "the value never reaches stdout or stderr: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("params: label"),
        "the evidence line records the parameter NAME: {stdout}"
    );

    // Every file beneath the harness root — the definition, the binding, the
    // state/audit database, the config and connections files, the session
    // state, and the report — is scanned for the value.
    let mut offenders = Vec::new();
    scan_for(&h.root, SENTINEL, &mut offenders);
    assert!(
        offenders.is_empty(),
        "the bound value must never persist on disk: {offenders:?}"
    );
    assert!(report.exists(), "the report was written");

    // The definition still holds the declaration, not the value.
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(!document.contains(SENTINEL), "{document}");
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(!binding.contains(SENTINEL), "{binding}");
    let _ = fs::remove_dir_all(h.root);
}

fn scan_for(dir: &Path, needle: &str, offenders: &mut Vec<PathBuf>) {
    let needle = needle.as_bytes();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            scan_for(&path, &String::from_utf8_lossy(needle), offenders);
        } else if let Ok(bytes) = fs::read(&path)
            && bytes.windows(needle.len()).any(|window| window == needle)
        {
            offenders.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// invariant 2: a value stays data, a typed null binds
// ---------------------------------------------------------------------------

/// A value carrying a quote and a SQL comment binds as a literal: zero rows,
/// never every row.
#[tokio::test]
async fn a_quote_and_comment_laden_value_stays_data() {
    let h = harness_with_database("injection");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );

    let output = h.run(&["investigation", "run", &id, "--param", "label=' OR 1=1 --"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    assert!(
        stdout.contains("\"row_count\":0") && stdout.contains("\"rows\":[]"),
        "the value matched nothing — it was data, not SQL: {stdout}"
    );
    assert!(
        !stdout.contains("first") && !stdout.contains("second"),
        "no seeded row leaked: {stdout}"
    );
    let _ = fs::remove_dir_all(h.root);
}
