//! S12 tests: the Markdown report built from the captured result — exact
//! layout, value neutralisation, bounds, and the atomic writer entry.

use super::super::write_report;
use super::*;
use crate::slash::MAX_REPORT_ROWS;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, SqlDialect,
};

const STARTED_UNIX_MS: i64 = 1_700_000_000_000;
const GENERATED_UNIX_MS: i64 = 1_700_000_005_000;

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".to_string(), "name".to_string()],
        rows: vec![
            serde_json::json!([1, "sentinel-alice"]),
            serde_json::json!([2, "bob, jr"]),
        ],
        row_count: 2,
        truncated: false,
        executed_sql: "SELECT id, name FROM users".to_string(),
    }
}

fn evidence(result: &QueryResult) -> ExecutionEvidence {
    ExecutionEvidence::for_result(
        result,
        ExecutionEvidenceArgs {
            execution_id: "xabc-1".to_string(),
            connection_label: "analytics".to_string(),
            connection_identity: None,
            dialect: SqlDialect::Sqlite,
            max_rows: 100,
            started_unix_ms: STARTED_UNIX_MS,
            finished_unix_ms: STARTED_UNIX_MS + 2_000,
            source: EvidenceSource::DirectSql,
        },
    )
}

fn render(rows: Option<usize>, result: &QueryResult) -> String {
    let evidence = evidence(result);
    render_report(&ReportInput {
        result,
        evidence: &evidence,
        rows,
        generated_unix_ms: GENERATED_UNIX_MS,
    })
    .expect("a small report renders")
}

/// The exact default document: header and generation line, the fenced SQL,
/// the provenance bullets, and a rows section that omits rows — no cell
/// values and nothing of the conversation.
#[test]
fn report_default_excludes_rows_and_history() {
    let result = sample_result();
    let report = render(None, &result);
    let expected = format!(
        "# saya report\n\nGenerated 2023-11-14 22:13:25 UTC by saya {}\n\n\
         ## Query\n\n```sql\nSELECT id, name FROM users\n```\n\n\
         ## Provenance\n\n- Connection label: analytics\n- Submitted SQL sha256: {}\n\
         - Execution id: xabc-1\n- Started: 2023-11-14 22:13:20 UTC\n\
         - Finished: 2023-11-14 22:13:22 UTC\n- Returned rows: 2\n- Row cap: 100\n\
         - Truncated: no\n- Scope: full result\n\n\
         ## Rows\n\nRows omitted (pass --rows N to include up to 100).\n",
        env!("CARGO_PKG_VERSION"),
        evidence(&result).submitted_sql_sha256,
    );
    assert_eq!(report, expected, "the default document is exactly this");
}

/// A truncated capture keeps both truncation statements in the report when
/// rows are included, and the provenance bullet carries it too.
#[test]
fn truncation_survives_report_export() {
    let result = QueryResult {
        columns: vec!["c1".to_string(), "c2".to_string()],
        rows: vec![
            serde_json::json!(["r1a", "r1b"]),
            serde_json::json!(["r2a", "r2b"]),
            serde_json::json!(["r3a", "r3b"]),
        ],
        row_count: 3,
        truncated: true,
        executed_sql: "SELECT 1".to_string(),
    };
    let report = render(Some(3), &result);
    assert!(
        report
            .contains("| c1 | c2 |\n| --- | --- |\n| r1a | r1b |\n| r2a | r2b |\n| r3a | r3b |\n"),
        "the table carries the captured rows: {report}"
    );
    assert!(
        report.contains("Showing 3 of 3 captured rows."),
        "the shown/captured summary is said: {report}"
    );
    assert!(
        report.contains("The query result itself was truncated at 100 rows."),
        "the truncation of the query itself is said: {report}"
    );
    assert!(
        report.contains("- Truncated: yes"),
        "the provenance bullet carries the truncation: {report}"
    );
}

/// Spreadsheet-formula, link, image, HTML, escape-character, pipe, and
/// control-character payloads are inert in the report: no `](`, no
/// `<script`, no ESC byte, pipes escaped so the column count is preserved,
/// control characters turned into spaces. Headers get the same treatment.
#[test]
fn csv_formula_and_markdown_control_payloads_are_inert() {
    let result = QueryResult {
        columns: vec!["c1".to_string(), "h|ack".to_string()],
        rows: vec![
            serde_json::json!(["=HYPERLINK(\"http://e\")", "ok1"]),
            serde_json::json!(["[x](http://e)", "ok2"]),
            serde_json::json!(["![i](u)", "ok3"]),
            serde_json::json!(["<script>", "ok4"]),
            serde_json::json!(["\u{1b}[31m", "ok5"]),
            serde_json::json!(["a|b", "ok6"]),
            serde_json::json!(["a\r\nb", "ok7"]),
            serde_json::json!(["a`b", "ok8"]),
        ],
        row_count: 8,
        truncated: false,
        executed_sql: "SELECT payload FROM t".to_string(),
    };
    let report = render(Some(8), &result);
    assert!(!report.contains("]("), "no link can form: {report}");
    assert!(
        !report.contains('\u{1b}'),
        "no escape byte survives: {report}"
    );
    // No raw HTML or autolink can form: every `<` and `>` in the report is
    // backslash-escaped (the SQL block here contains neither).
    let unescaped = |open: char| {
        report
            .match_indices(open)
            .filter(|(index, _)| !report[..*index].ends_with('\\'))
            .count()
    };
    assert_eq!(unescaped('<'), 0, "every `<` is escaped: {report}");
    assert_eq!(unescaped('>'), 0, "every `>` is escaped: {report}");
    for expected in [
        "| c1 | h\\|ack |",
        "| --- | --- |",
        "| =HYPERLINK\\(\"http://e\"\\) | ok1 |",
        "| \\[x\\]\\(http://e\\) | ok2 |",
        "| \\!\\[i\\]\\(u\\) | ok3 |",
        "| \\<script\\> | ok4 |",
        "|  \\[31m | ok5 |",
        "| a\\|b | ok6 |",
        "| a  b | ok7 |",
        "| a\\`b | ok8 |",
    ] {
        assert!(
            report.contains(expected),
            "the payload row renders inert {expected:?}: {report}"
        );
    }
}

/// The SQL block protects itself: the fence is always longer than any
/// backtick run inside the SQL (a run of one still needs the 3-backtick
/// minimum), and control characters other than `\n` and `\t` are replaced
/// while the SQL otherwise goes in verbatim.
#[test]
fn sql_fence_survives_backticks_in_sql() {
    // A one-backtick run: the minimum 3-backtick fence already outruns it.
    let result = QueryResult::empty("SELECT '`tick`' FROM t");
    let report = render(None, &result);
    for line in ["```sql", "SELECT '`tick`' FROM t", "```"] {
        assert!(
            report.lines().any(|l| l == line),
            "the fenced block carries the backtick SQL: {report}"
        );
    }
    assert!(
        !report.lines().any(|l| l.starts_with("````")),
        "no 4-backtick fence is needed for a one-backtick run: {report}"
    );

    // A three-backtick run forces a four-backtick fence.
    let result = QueryResult::empty("SELECT '```x' FROM t");
    let report = render(None, &result);
    for line in ["````sql", "SELECT '```x' FROM t", "````"] {
        assert!(
            report.lines().any(|l| l == line),
            "a three-backtick run forces a four-backtick fence: {report}"
        );
    }
    assert!(
        !report.lines().any(|l| l == "```sql" || l == "```"),
        "no 3-backtick fence line may remain: {report}"
    );

    let result = QueryResult::empty("SELECT 'a\tb'\r\nFROM t");
    let report = render(None, &result);
    assert!(
        report.contains("```sql\nSELECT 'a\tb' \nFROM t\n```"),
        "tab and newline survive, CR does not: {report}"
    );
}

/// The renderer clamps rows to the shared 100-row bound the parser refuses
/// above, and a document over the 2 MiB ceiling is refused while building.
#[test]
fn report_rows_bound_and_size_cap() {
    let result = QueryResult {
        columns: vec!["c1".to_string()],
        rows: (0..150)
            .map(|i| serde_json::json!([format!("r{i}")]))
            .collect(),
        row_count: 150,
        truncated: true,
        executed_sql: "SELECT c1 FROM t".to_string(),
    };
    let report = render(Some(500), &result);
    assert!(
        report.contains("Showing 100 of 150 captured rows."),
        "the bound clamps at the renderer: {report}"
    );
    assert!(report.contains("| r99 |"), "row 99 is included: {report}");
    assert!(
        !report.contains("| r100 |"),
        "row 100 is cut by the bound: {report}"
    );

    let error = {
        let evidence = evidence(&sample_result());
        render_report_with_ceiling(
            &ReportInput {
                result: &sample_result(),
                evidence: &evidence,
                rows: Some(1),
                generated_unix_ms: GENERATED_UNIX_MS,
            },
            64,
        )
        .expect_err("a report over the injected ceiling is refused")
    };
    assert_eq!(
        error, "report larger than 2 MiB; include fewer rows",
        "the refusal is in the report's own words"
    );
}

/// The production bounds are the ones the invariant names: 2 MiB document,
/// 100 rows.
#[test]
fn the_report_bounds_are_2_mib_and_100_rows() {
    assert_eq!(MAX_REPORT_BYTES, 2 * 1024 * 1024);
    assert_eq!(MAX_REPORT_ROWS, 100);
}

/// One wide cell cannot blow up the document or the table layout: it is cut
/// at 200 characters with an ellipsis.
#[test]
fn long_cells_are_truncated_with_an_ellipsis() {
    let result = QueryResult {
        columns: vec!["c1".to_string()],
        rows: vec![serde_json::json!(["x".repeat(250)])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT c1".to_string(),
    };
    let report = render(Some(1), &result);
    let cut = "x".repeat(200);
    assert!(
        report.contains(&format!("| {cut}… |")),
        "the cell is cut at 200 characters with an ellipsis: {report}"
    );
    assert!(
        !report.contains(&"x".repeat(201)),
        "the full 250-character value never lands: {report}"
    );
}

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

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-report-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("report test dir");
    dir
}
