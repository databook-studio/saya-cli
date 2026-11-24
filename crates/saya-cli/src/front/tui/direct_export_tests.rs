//! The new surface's `/export` (`03` §4.1): one parser, and the dispatch the
//! line REPL already makes — a Markdown destination is today's report, a data
//! destination is today's data writer. Every test but the last reads a result
//! the surface holds, so a write is checked without querying a database.

use std::path::Path;

use saya_types::{EvidenceSource, QueryResult};

use super::direct::{self, DirectOutcome};
use super::direct_fixture::{Fixture, held_preview, report_evidence, report_result, submit};
use super::direct_run;
use super::focus::EntryPath;
use super::msg::{Cmd, Msg};
use super::update::update;
use crate::front::{Entry, ResultPreview};
use crate::interactive::tui::export::write_report;

/// Performs one command on the launch's own dependencies and folds its answer
/// back in, as the runtime does.
fn answer(model: &mut super::model::Model, fixture: &Fixture, command: &Cmd) {
    let Cmd::Direct(command) = command else {
        panic!("a direct command was expected, got {command:?}");
    };
    let worker = direct_run::perform(&fixture.deps, command.clone());
    update(model, Msg::Worker(worker));
}

/// Submits one line whose answer is one command, and performs it.
fn run_line(model: &mut super::model::Model, fixture: &Fixture, line: &str) {
    let cmds = submit(model, line);
    assert_eq!(cmds.len(), 1, "`{line}` is one command, never a turn");
    answer(model, fixture, &cmds[0]);
}

/// The newest result entry the transcript shows.
fn preview(model: &super::model::Model) -> &ResultPreview {
    model
        .turns()
        .iter()
        .rev()
        .flat_map(|turn| turn.entries().iter().rev())
        .find_map(|entry| match entry {
            Entry::ResultPreview(preview) => Some(preview),
            _ => None,
        })
        .expect("the transcript holds a result entry")
}

/// A report compared byte for byte except the one line naming when it was
/// written: two writes can straddle a second.
fn without_generated_line(document: &str) -> String {
    document
        .lines()
        .filter(|line| !line.starts_with("Generated "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn export_md_writes_the_report_the_line_repl_writes() {
    let fixture = Fixture::new("export-md");
    let mut model = fixture.model();
    direct::land(&mut model, DirectOutcome::Result(held_preview()));

    let path = fixture.root.join("out.md");
    run_line(&mut model, &fixture, &format!("/export {}", path.display()));
    let written = std::fs::read_to_string(&path).expect("a .md destination writes the report");

    // The line REPL's `/export out.md` is one call to the report writer with
    // the captured result and its evidence, pinned here into a second path.
    let expected_path = fixture.root.join("expected.md");
    write_report(
        &report_result(),
        &report_evidence(),
        None,
        &expected_path,
        false,
    )
    .expect("the line REPL's writer writes the fixture report");
    let expected = std::fs::read_to_string(&expected_path).expect("the report was written");
    assert_eq!(
        without_generated_line(&written),
        without_generated_line(&expected),
        "the new surface writes the bytes the line REPL's /export writes"
    );
    for pinned in [
        "# saya report",
        "## Query",
        "SELECT id, label FROM events ORDER BY id",
        "## Provenance",
        "Submitted SQL sha256:",
        "Rows omitted (pass --rows N to include up to 100).",
    ] {
        assert!(written.contains(pinned), "{pinned} is in {written}");
    }
}

#[test]
fn export_format_md_writes_the_report_whatever_the_extension_says() {
    let fixture = Fixture::new("export-format-md");
    let mut model = fixture.model();
    direct::land(&mut model, DirectOutcome::Result(held_preview()));

    let path = fixture.root.join("out.txt");
    run_line(
        &mut model,
        &fixture,
        &format!("/export --format md {}", path.display()),
    );
    let written = std::fs::read_to_string(&path).expect("--format md writes the report");
    assert!(
        written.starts_with("# saya report\n\n"),
        "a stated format wins over the destination's extension: {written}"
    );
    assert!(
        written.contains("## Provenance") && !written.contains("id,label"),
        "the document is the report, not the rows: {written}"
    );
}

#[test]
fn export_rows_bounds_the_report_table() {
    let fixture = Fixture::new("export-rows");
    let mut model = fixture.model();
    direct::land(&mut model, DirectOutcome::Result(held_preview()));

    let bounded = fixture.root.join("one-row.md");
    run_line(
        &mut model,
        &fixture,
        &format!("/export --rows 1 {}", bounded.display()),
    );
    let table = std::fs::read_to_string(&bounded).expect("the report was written");
    assert!(
        table.contains("Showing 1 of 2 captured rows."),
        "--rows bounds the report's table: {table}"
    );
    assert!(
        table.contains("| 1 | one |") && !table.contains("| 2 | two |"),
        "the table stops at the bound: {table}"
    );
}

#[test]
fn export_writes_the_focused_result_not_the_newest() {
    let fixture = Fixture::new("export-focused");
    let mut model = fixture.model();
    direct::land(&mut model, DirectOutcome::Result(held_preview()));
    let mut newer = held_preview();
    newer.sql = "SELECT 2".to_owned();
    newer.result = QueryResult::empty("SELECT 2");
    direct::land(&mut model, DirectOutcome::Result(newer));
    // The block cursor holds the first result. While it does, the keyboard is
    // the cursor's, so the line is read here and performed as the runtime
    // would perform it.
    model.focus = Some(EntryPath::new(0, 0));

    let path = fixture.root.join("focused.md");
    let super::direct::Read::Cmd(command) =
        direct::read(&model, &format!("/export {}", path.display()))
    else {
        panic!("the cursor's result is the one /export writes");
    };
    answer(&mut model, &fixture, &command);
    let written = std::fs::read_to_string(&path).expect("the report was written");
    assert!(
        written.contains("SELECT id, label FROM events ORDER BY id")
            && !written.contains("SELECT 2"),
        "the cursor's result is the one written: {written}"
    );
}

#[test]
fn a_refused_capture_says_why_and_writes_nothing() {
    let fixture = Fixture::new("export-refused");
    let mut model = fixture.model();
    let mut refused = held_preview();
    refused.evidence = None;
    refused.refusal = Some(crate::agent::tools::CaptureRefusalReason::ModelViewTruncated);
    direct::land(&mut model, DirectOutcome::Result(refused));

    let path = fixture.root.join("refused.md");
    let cmds = submit(&mut model, &format!("/export {}", path.display()));
    assert!(cmds.is_empty(), "nothing is written: no command: {cmds:?}");
    let said = model.toast().expect("the surface says why");
    assert!(
        said.contains("model saw a truncated result"),
        "the refusal's own words say why: {said}"
    );
    assert!(
        !Path::new(&path).exists(),
        "a refused capture writes no file"
    );
}

#[test]
fn a_select_reports_the_execution_it_ran() {
    let fixture = Fixture::new("export-provenance");
    let mut model = fixture.model();
    run_line(
        &mut model,
        &fixture,
        "/sql SELECT id, label FROM events ORDER BY id",
    );
    let held = preview(&model);
    let evidence = held
        .evidence
        .clone()
        .expect("the worker's envelope names the execution it ran");
    assert_eq!(
        evidence.source,
        EvidenceSource::DirectSql,
        "a /sql result's report states a direct execution"
    );
    assert_eq!(evidence.connection_label, "local");

    let path = fixture.root.join("provenance.md");
    run_line(&mut model, &fixture, &format!("/export {}", path.display()));
    let written = std::fs::read_to_string(&path).expect("the report was written");
    assert!(
        written.contains(&format!("Execution id: {}", evidence.short_id())),
        "the report states the execution the rows came from: {written}"
    );
}
