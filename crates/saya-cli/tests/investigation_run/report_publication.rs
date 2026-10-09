#[tokio::test]
async fn run_report_writes_sql_and_provenance_without_rows() {
    let h = harness_with_database("report-default");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    let output = h.run(&[
        "investigation",
        "run",
        &id,
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
    let document = fs::read_to_string(&report).unwrap();
    assert!(document.contains("# saya report"), "{document}");
    assert!(document.contains("## Query"), "{document}");
    assert!(
        document.contains("SELECT id, label FROM events ORDER BY id"),
        "the exact saved SQL is in the report: {document}"
    );
    assert!(document.contains("## Provenance"), "{document}");
    assert!(document.contains("Connection label: local"), "{document}");
    assert!(document.contains("Submitted SQL sha256: "), "{document}");
    assert!(document.contains("Execution id: "), "{document}");
    assert!(document.contains("Scope: full result"), "{document}");
    assert!(
        document.contains("Rows omitted (pass --rows N to include up to 100)."),
        "{document}"
    );
    assert!(
        !document.contains("| id |"),
        "rows land only on --rows N: {document}"
    );
    assert!(
        !document.contains("| 1 | first |"),
        "no row data without --rows: {document}"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn run_report_rows_opt_in() {
    let h = harness_with_database("report-rows");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report.to_str().unwrap(),
        "--rows",
        "1",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let document = fs::read_to_string(&report).unwrap();
    assert!(document.contains("| id | label |"), "{document}");
    assert!(document.contains("| 1 | first |"), "{document}");
    assert!(
        !document.contains("| 2 | second |"),
        "only the requested rows land: {document}"
    );
    assert!(
        document.contains("Showing 1 of 2 captured rows."),
        "{document}"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn stale_run_writes_no_report() {
    let h = harness_with_database("report-stale");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    // The definition is revised on disk (revision 2, binding says 1).
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    fs::write(
        h.document_path(&id),
        document.replace("\"revision\": 1", "\"revision\": 2"),
    )
    .unwrap();

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report.to_str().unwrap(),
        "--rows",
        "2",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    assert!(
        h.stderr(&output).contains("revision changed"),
        "the refusal names what changed: {}",
        h.stderr(&output)
    );
    assert!(!report.exists(), "a refused run creates no report file");
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn run_report_refuses_existing_without_overwrite() {
    let h = harness_with_database("report-existing");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");
    let report_arg = report.to_str().unwrap();

    let first = h.run(&["investigation", "run", &id, "--report", report_arg]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&first),
        h.stderr(&first)
    );
    // Mark the file so an accidental clobber is detectable.
    fs::write(&report, b"sentinel").unwrap();

    let refused = h.run(&["investigation", "run", &id, "--report", report_arg]);
    assert_eq!(refused.status.code(), Some(2), "{}", h.stderr(&refused));
    assert!(
        h.stderr(&refused).contains("exists; add --overwrite"),
        "the refusal names the way out: {}",
        h.stderr(&refused)
    );
    assert_eq!(
        fs::read(&report).unwrap(),
        b"sentinel",
        "the existing report is untouched"
    );
    assert!(
        h.binding_path(&id).exists(),
        "the S7 binding rules are unaffected by the report refusal"
    );

    let replaced = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report_arg,
        "--overwrite",
    ]);
    assert_eq!(
        replaced.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&replaced),
        h.stderr(&replaced)
    );
    assert!(
        fs::read_to_string(&report)
            .unwrap()
            .contains("# saya report"),
        "the report replaced the old file"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn report_flags_without_report_are_usage_errors() {
    let h = harness_with_database("report-usage");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    let rows_only = h.run(&["investigation", "run", &id, "--rows", "2"]);
    assert_eq!(rows_only.status.code(), Some(2), "{}", h.stderr(&rows_only));
    assert!(
        h.stderr(&rows_only).contains("--rows requires --report"),
        "{}",
        h.stderr(&rows_only)
    );

    let overwrite_only = h.run(&["investigation", "run", &id, "--overwrite"]);
    assert_eq!(
        overwrite_only.status.code(),
        Some(2),
        "{}",
        h.stderr(&overwrite_only)
    );
    assert!(
        h.stderr(&overwrite_only)
            .contains("--overwrite requires --report"),
        "{}",
        h.stderr(&overwrite_only)
    );

    let over_cap = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report.to_str().unwrap(),
        "--rows",
        "101",
    ]);
    assert_eq!(over_cap.status.code(), Some(2), "{}", h.stderr(&over_cap));
    assert!(
        h.stderr(&over_cap).contains("--rows is capped at 100"),
        "{}",
        h.stderr(&over_cap)
    );
    assert!(!report.exists(), "a usage error writes nothing");
    assert!(
        !h.stdout(&over_cap).contains("\"event\":\"query_result\""),
        "a usage error never executes: {}",
        h.stdout(&over_cap)
    );
    let _ = fs::remove_dir_all(h.root);
}
