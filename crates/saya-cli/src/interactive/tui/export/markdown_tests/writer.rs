/// The writer publishes exactly the rendered report and returns the rows it
/// included. (The Generated line carries the wall clock at write time; its
/// formatting is pinned by the default-document test, so it is stripped
/// before comparing.)
#[test]
fn write_report_writes_the_rendered_report_and_counts_rows() {
    let dir = temp_dir("report-write");
    let path = dir.join("report.md");
    let result = sample_result();
    let included = write_report(&result, &evidence(&result), Some(2), &path, false)
        .expect("the report writes");
    assert_eq!(included, Some(2), "the two captured rows are included");
    let written = std::fs::read_to_string(&path).expect("the file exists");
    let strip_generated = |report: &str| {
        report
            .lines()
            .filter(|line| !line.starts_with("Generated "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        strip_generated(&written),
        strip_generated(&render(Some(2), &result)),
        "the file is exactly the rendered report"
    );
    assert!(
        std::fs::read_to_string(&path)
            .expect("the file exists")
            .starts_with("# saya report\n\nGenerated "),
        "the file opens with the header and generation line"
    );
}

/// With no row request the writer reports the omission to the caller.
#[test]
fn write_report_rows_omitted_returns_none() {
    let dir = temp_dir("report-omit");
    let path = dir.join("report.md");
    let result = sample_result();
    let included =
        write_report(&result, &evidence(&result), None, &path, false).expect("the report writes");
    assert_eq!(included, None, "no rows were requested");
    assert!(
        !std::fs::read_to_string(&path)
            .expect("the file exists")
            .contains("sentinel-alice"),
        "no cell values land without --rows"
    );
}

/// The writer keeps the S11 destination guard: an existing file needs
/// `--overwrite`, and the refusal leaves it byte-for-byte.
#[test]
fn write_report_refuses_an_existing_destination() {
    let dir = temp_dir("report-existing");
    let path = dir.join("report.md");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    let result = sample_result();
    let error = write_report(&result, &evidence(&result), None, &path, false)
        .expect_err("an existing destination is refused");
    assert!(
        error.contains("exists; add --overwrite"),
        "the refusal names the flag: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("destination readable"),
        "keep-me",
        "the existing file is untouched"
    );
}
