//! The two copies every submitted line leaves behind (`02` §7): the ring's
//! `input_history` file, and the transcript's echo of the operator's own line —
//! both redacted, so an investigation `--param` value neither persists nor
//! echoes.
//!
//! The ring's own redaction is `interactive::tui::history`'s and is pinned there
//! (`ring_redact_tests`, `flags_tests`); these tests pin the two writes the new
//! composer adds, on a ring that persists to a file of the test's own.

use std::path::{Path, PathBuf};

use super::model::Model;
use super::msg::Cmd;
use super::update::update;
use super::update_fixture::enter;
use crate::front::Entry;
use crate::interactive::tui::History;

/// The submitted parameter value: synthetic, and shaped so only the `--param`
/// redaction can keep it off disk (the generic redaction alone never would).
const PARAM_VALUE: &str = "synthetic-confidential-customer-922";

/// One history file of this test run, under the build's own `target/`: a test
/// tree writes nothing the workspace would commit.
fn history_path(tag: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|workspace| workspace.parent())
        .expect("the crate sits inside the workspace")
        .join("target/tmp-history");
    std::fs::create_dir_all(&root).expect("the history test tree is made");
    root.join(format!("tx4-{tag}-{}.txt", std::process::id()))
}

/// A model whose ring persists to `path`, and to nothing else.
fn model_with(path: &Path) -> Model {
    Model {
        history: History::with_path(path.to_path_buf()),
        ..Model::default()
    }
}

/// One line, typed whole and submitted by Enter — the path every submitted line
/// takes through the surface.
fn submitted(model: &mut Model, line: &str) -> Vec<Cmd> {
    model.insert(line);
    update(model, enter())
}

/// The operator's own lines the transcript echoes, oldest first.
fn echoes(model: &Model) -> Vec<String> {
    model
        .turns()
        .iter()
        .flat_map(|turn| turn.entries())
        .filter_map(|entry| match entry {
            Entry::UserMessage(message) => Some(message.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn every_submitted_line_is_pushed_and_persisted() {
    let path = history_path("push");
    let mut model = model_with(&path);

    let cmds = submitted(&mut model, "count the orders");

    assert_eq!(
        cmds,
        vec![Cmd::Send("count the orders".to_owned())],
        "the line is still a turn's"
    );
    assert_eq!(
        model.history().search("count the orders"),
        vec!["count the orders"],
        "the submitted line is in the ring"
    );
    let file = std::fs::read_to_string(&path).expect("the submitted line was persisted");
    assert!(
        file.contains("count the orders"),
        "the ring's file keeps the line: {file:?}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_line_that_runs_nothing_is_still_the_rings() {
    let path = history_path("refused");
    let mut model = model_with(&path);

    let cmds = submitted(&mut model, "/investigation run");

    assert!(cmds.is_empty(), "a refused spelling runs nothing: {cmds:?}");
    let file = std::fs::read_to_string(&path).expect("the submitted line was persisted");
    assert!(
        file.contains("/investigation run"),
        "a line the surface answers itself is recalled too: {file:?}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_param_value_neither_persists_nor_echoes() {
    let path = history_path("param");
    let mut model = model_with(&path);
    let line = format!("/investigation run inv-1 --param label={PARAM_VALUE}");

    let cmds = submitted(&mut model, &line);

    assert!(
        matches!(cmds.as_slice(), [Cmd::Direct(_)]),
        "an investigation run is the surface's own command: {cmds:?}"
    );
    let file = std::fs::read_to_string(&path).expect("the submitted line was persisted");
    assert!(
        file.contains("--param label=…"),
        "the history keeps the redacted form: {file:?}"
    );
    assert!(
        !file.contains(PARAM_VALUE),
        "the value must not persist in input_history: {file:?}"
    );
    let echoed = echoes(&model);
    assert!(
        echoed.iter().any(|line| line.contains("--param label=…")),
        "the transcript echoes the redacted form: {echoed:?}"
    );
    assert!(
        !echoed.iter().any(|line| line.contains(PARAM_VALUE)),
        "the value must not echo in the transcript: {echoed:?}"
    );
    let _ = std::fs::remove_file(&path);
}
