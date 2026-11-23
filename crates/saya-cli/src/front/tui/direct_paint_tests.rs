//! A completed direct command on the frame (`02` §3.3): its rows are the
//! transcript's, above the status line and the composer — never a status row
//! they share with the surface's own words.
//!
//! The command is performed on the fixture's real dependencies and its answer is
//! folded back through `Msg::Worker`, exactly as the runtime hands it over; each
//! frame is painted through the real panes onto ratatui's test backend, so what
//! these tests read is the screen.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::direct_fixture::{Fixture, submit};
use super::direct_run::{self, DirectDeps};
use super::model::Model;
use super::msg::{Cmd, Msg, Worker};
use super::run_draw::{Facts, frame};
use super::update::update;
use crate::front::Entry;

/// The investigation the run replays: a saved query over the fixture's own
/// table, so the run's result is the one row `match_id = 2`.
const SAVE: &str =
    "/investigation save by-label --sql SELECT id AS match_id FROM events WHERE label = 'two'";

/// The id of the one document the save wrote, as the repository names it.
fn saved_id(deps: &DirectDeps) -> String {
    let runtime = deps.runtime.current();
    let root = &runtime.investigations_root;
    let mut ids: Vec<String> = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("the fixture's investigations root {root:?}: {error}"))
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| {
            entry
                .path()
                .file_stem()
                .expect("a saved document is a file")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    ids.sort();
    assert_eq!(
        ids.len(),
        1,
        "one document was saved under {root:?}: {ids:?}"
    );
    ids.remove(0)
}

/// Submits one line, performs the one command it decided on, and folds the
/// answer back in — the path the runtime takes, with no terminal in it.
fn perform_line(model: &mut Model, deps: &DirectDeps, line: &str) -> Worker {
    let cmds = submit(model, line);
    let [Cmd::Direct(command)] = cmds.as_slice() else {
        panic!("{line} is one direct command, got {cmds:?}");
    };
    let worker = direct_run::perform(deps, command.clone());
    update(model, Msg::Worker(worker.clone()));
    worker
}

/// The newest turn's notice, once one carries the run's rows.
fn notice(model: &Model) -> Option<String> {
    model
        .turns()
        .iter()
        .rev()
        .flat_map(|turn| turn.entries().iter().rev())
        .find_map(|entry| match entry {
            Entry::Notice(notice) => Some(notice.message.clone()),
            _ => None,
        })
}

/// The model a settled `/investigation run` leaves, the evidence line the run
/// ended in, and the line the operator submitted — the transcript's first row:
/// the document is saved first, on a model of its own, so the frame under test
/// holds the run and nothing else.
fn settled(label: &str) -> (Model, String, String) {
    let fixture = Fixture::new(label);
    let deps = fixture.investigation_deps();
    perform_line(&mut fixture.model(), &deps, SAVE);
    let id = saved_id(&deps);
    let mut model = fixture.model();
    let line = format!("/investigation run {id}");
    perform_line(&mut model, &deps, &line);
    let evidence = notice(&model)
        .and_then(|message| message.lines().last().map(str::to_owned))
        .unwrap_or_default();
    (model, evidence, line)
}

/// The screen `model` paints at `width × height`, one row per line, trailing
/// air trimmed so an assertion reads as the row an operator sees.
fn painted(model: &Model, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, &Facts::default(), area))
        .expect("the frame is painted");
    let screen = format!("{}", terminal.backend());
    screen
        .lines()
        .map(|row| row.trim_matches('"').trim_end().to_owned())
        .collect()
}

#[test]
fn a_completed_investigation_run_lands_its_rows_in_the_transcript() {
    let (model, evidence, _line) = settled("paint-entry");
    let notice = notice(&model).unwrap_or_else(|| {
        panic!(
            "the run's rows are a transcript entry, not the surface's transient \
             word — the newest turn holds {:?}",
            model.turns()
        )
    });
    assert!(
        notice.starts_with("match_id\n2\n"),
        "the entry opens with the run's rows, got {notice:?}"
    );
    assert!(
        evidence.starts_with("saved investigation: local · 1 rows · exec "),
        "and ends in the run's evidence line, got {evidence:?}"
    );
    assert!(
        model.toast().is_none(),
        "a completed command's rows never live on the status row, got {:?}",
        model.toast()
    );
}

#[test]
fn the_run_rows_are_above_the_status_line_at_100x30() {
    let (model, evidence, line) = settled("paint-100");
    let rows = painted(&model, 100, 30);
    assert_eq!(rows.len(), 30, "every row of the terminal is painted");
    assert_eq!(
        rows[..5],
        [
            format!("   ❯ {line}"),
            String::new(),
            String::from("   match_id"),
            String::from("   2"),
            format!("   {evidence}"),
        ],
        "the operator's line opens the turn, then the run's rows, intact and one line each: {rows:#?}"
    );
    assert!(
        rows[5..26].iter().all(String::is_empty),
        "nothing else is painted under them: {rows:#?}"
    );
    assert_eq!(
        rows[26..],
        [
            String::from(
                " ask │ local │ no workspace                                                                  ? help"
            ),
            String::from(
                " ╭ Message ───────────────────────────────────────────────────────────────────────────────────────╮"
            ),
            String::from(
                " │ Ask about local or describe a change                                                           │"
            ),
            String::from(
                " ╰ Enter send · Ctrl+O detail · Ctrl+K commands ──────────────────────────────────────────────────╯"
            ),
        ],
        "the status line keeps its own row, and the composer closes the screen under it: {rows:#?}"
    );
}

#[test]
fn the_run_rows_are_above_the_status_line_at_80x24() {
    let (model, evidence, line) = settled("paint-80");
    let rows = painted(&model, 80, 24);
    assert_eq!(rows.len(), 24, "every row of the terminal is painted");
    assert_eq!(
        rows[..5],
        [
            format!("   ❯ {line}"),
            String::new(),
            String::from("   match_id"),
            String::from("   2"),
            format!("   {evidence}"),
        ],
        "the operator's line opens the turn, then the run's rows, intact and one line each: {rows:#?}"
    );
    assert!(
        rows[5..20].iter().all(String::is_empty),
        "nothing else is painted under them: {rows:#?}"
    );
    assert_eq!(
        rows[20..],
        [
            String::from(
                " ask │ local │ no workspace                                              ? help"
            ),
            String::from(
                " ╭ Message ───────────────────────────────────────────────────────────────────╮"
            ),
            String::from(
                " │ Ask about local or describe a change                                       │"
            ),
            String::from(
                " ╰ Enter send · Ctrl+O detail · Ctrl+K commands ──────────────────────────────╯"
            ),
        ],
        "the status line keeps its own row, and the composer closes the screen under it: {rows:#?}"
    );
}
