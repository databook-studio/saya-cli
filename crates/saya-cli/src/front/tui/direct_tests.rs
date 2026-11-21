//! The direct commands a run's operator types (`03` §4.1), end to end: the
//! line in through the pure update, the command performed on the launch's own
//! dependencies, and the answer folded back through `Msg::Worker` — the path
//! the runtime takes, with no terminal in it.

use std::path::Path;

use super::direct::{Direct, NOTHING_TO_CHART};
use super::direct_fixture::{Fixture, submit};
use super::direct_run;
use super::msg::{Cmd, Msg, Worker};
use super::update::update;
use crate::front::Entry;
use crate::interactive::tui::write_result_overwrite;

/// The newest turn's entries, as the transcript shows them.
fn entries(model: &super::model::Model) -> Vec<&Entry> {
    model
        .turns()
        .last()
        .map(|turn| turn.entries().iter().collect())
        .unwrap_or_default()
}

/// The newest result entry's preview, or a failure naming what was there.
fn preview(model: &super::model::Model) -> &crate::front::ResultPreview {
    entries(model)
        .into_iter()
        .rev()
        .find_map(|entry| match entry {
            Entry::ResultPreview(preview) => Some(preview),
            _ => None,
        })
        .expect("the transcript holds a result entry")
}

/// Runs one direct command through the same worker the port would use and
/// folds its answer back in, as the runtime does.
fn answer(model: &mut super::model::Model, fixture: &Fixture, command: &Direct) {
    let worker = direct_run::perform(&fixture.deps, command.clone());
    update(model, Msg::Worker(worker));
}

#[test]
fn a_select_lands_its_rows_as_a_result_entry() {
    let fixture = Fixture::new("select");
    let mut model = fixture.model();
    let cmds = submit(&mut model, "/sql SELECT id, label FROM events ORDER BY id");
    assert_eq!(
        cmds.len(),
        1,
        "a direct command is one command, never a turn"
    );
    let Cmd::Direct(command) = &cmds[0] else {
        panic!("`/sql` is a direct command, got {:?}", cmds[0]);
    };
    answer(&mut model, &fixture, command);

    let preview = preview(&model);
    assert_eq!(preview.sql, "SELECT id, label FROM events ORDER BY id");
    assert_eq!(
        preview.result.row_count, 2,
        "the fixture's two rows came back"
    );
    assert_eq!(
        preview.result.rows,
        vec![serde_json::json!([1, "one"]), serde_json::json!([2, "two"]),],
        "the rows are the fixture's, in order"
    );
    assert!(
        preview.refusal.is_none(),
        "a held result has no refusal under it"
    );
}

#[test]
fn a_drop_is_refused_by_the_safety_layer_and_renders_the_refusal() {
    let fixture = Fixture::new("refused");
    let mut model = fixture.model();
    let cmds = submit(&mut model, "/sql DROP TABLE events");
    let Cmd::Direct(command) = &cmds[0] else {
        panic!("`/sql` is a direct command, got {:?}", cmds[0]);
    };
    answer(&mut model, &fixture, command);

    assert!(
        !entries(&model)
            .into_iter()
            .any(|entry| matches!(entry, Entry::ResultPreview(_))),
        "a refused statement never lands as rows"
    );
    let refusal = entries(&model)
        .into_iter()
        .rev()
        .find_map(|entry| match entry {
            Entry::Error(error) => Some(error.message.clone()),
            _ => None,
        })
        .expect("the refusal is a line of its own");
    assert!(
        refusal.contains("query rejected by read-only safety policy"),
        "the refusal is the safety layer's own words, got {refusal:?}"
    );
    assert!(
        refusal.contains("DROP"),
        "the refusal names the statement it refused, got {refusal:?}"
    );
}

#[test]
fn an_export_writes_the_bytes_todays_export_writes() {
    let fixture = Fixture::new("export");
    let mut model = fixture.model();
    let cmds = submit(&mut model, "/sql SELECT id, label FROM events ORDER BY id");
    let Cmd::Direct(command) = &cmds[0] else {
        panic!("`/sql` is a direct command, got {:?}", cmds[0]);
    };
    answer(&mut model, &fixture, command);
    let result = preview(&model).result.clone();

    let path = fixture.root.join("out.csv");
    let cmds = submit(&mut model, &format!("/export {}", path.display()));
    assert_eq!(cmds.len(), 1, "`/export` is one command, never a turn");
    let Cmd::Direct(command) = &cmds[0] else {
        panic!("`/export` is a direct command, got {:?}", cmds[0]);
    };
    answer(&mut model, &fixture, command);

    let written = std::fs::read(&path).expect("the export wrote the file");
    let today = fixture.root.join("today.csv");
    write_result_overwrite(&result, Path::new(&today), false).expect("today's export writes");
    assert_eq!(
        written,
        std::fs::read(&today).expect("today's export wrote the file"),
        "a new-surface export is byte for byte today's export"
    );
    assert!(
        !written.is_empty(),
        "the export wrote the fixture's rows, not nothing"
    );
}

#[test]
fn a_chart_with_no_result_says_so_in_a_toast() {
    let fixture = Fixture::new("chart");
    let mut model = fixture.model();
    let cmds = submit(&mut model, "/chart");
    assert!(cmds.is_empty(), "nothing is performed: no command");
    assert_eq!(
        model.toast(),
        Some(NOTHING_TO_CHART),
        "the surface says there is nothing to chart, in today's words"
    );
    assert!(
        entries(&model).is_empty(),
        "nothing was written into the transcript"
    );
}

#[test]
fn a_save_with_no_query_keeps_todays_words() {
    let fixture = Fixture::new("save");
    let mut model = fixture.model();
    let cmds = submit(&mut model, "/investigation save revenue");
    let Cmd::Direct(command) = &cmds[0] else {
        panic!("`/investigation` is a direct command, got {:?}", cmds[0]);
    };
    let worker: Worker = direct_run::perform(&fixture.deps, command.clone());
    assert_eq!(
        worker.message, "Nothing to save yet: run a query first (or pass --sql).",
        "the fill refuses in the words today's TUI uses"
    );
    assert!(
        fixture.trust.bound().is_empty(),
        "nothing was bound by a save"
    );
}
