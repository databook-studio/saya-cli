//! The status row names the session, not the launch: after `/model X`,
//! `/provider X` or `/connect P` the row states the model and the database the
//! session is on now, and the launch's own are gone.
//!
//! The facts reach the surface as one message the update owns
//! ([`Msg::Restated`]), sent by the port that performed the command, so the row
//! is asserted where the operator reads it: on the painted frame.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::model::Model;
use super::msg::Msg;
use super::run_draw::{Facts, frame};
use super::run_front::Forward;
use super::update::update;

/// The screen `model` paints at `width × height`.
fn painted(model: &Model, facts: &Facts, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, facts, area))
        .expect("the frame is painted");
    format!("{}", terminal.backend())
        .lines()
        .map(|row| row.trim_matches('"').to_owned())
        .collect()
}

/// The row the status line occupies.
fn status(rows: &[String]) -> &String {
    rows.iter()
        .find(|row| row.contains("? help") || row.contains("working") || row.contains("queued 1"))
        .unwrap_or_else(|| panic!("no status row in {rows:#?}"))
}

/// The launch's facts: the model the process started on.
fn launch() -> Facts {
    Facts {
        model: String::from("deepseek/deepseek-v4.1-flash"),
        ..Facts::default()
    }
}

/// A session talking to the demo database, as the launch opened it.
fn session() -> Model {
    Model {
        database: Some(String::from("demo")),
        ..Model::default()
    }
}

/// One command's restatement, as the port sends it.
fn restated(model: &str, database: &str) -> Msg {
    Msg::Restated {
        model: model.to_owned(),
        database: Some(database.to_owned()),
    }
}

#[test]
fn the_launch_model_is_named_until_a_command_states_another() {
    for (width, height) in [(100, 30), (60, 24)] {
        let rows = painted(&session(), &launch(), width, height);
        assert!(
            status(&rows).contains("deepseek-v4.1-flash │ demo"),
            "{width}x{height}: the launch's model and database: {:?}",
            status(&rows)
        );
    }
}

#[test]
fn a_restated_model_is_named_on_the_status_row() {
    let mut model = session();
    update(&mut model, restated("other", "demo"));

    let rows = painted(&model, &launch(), 100, 30);
    let row = status(&rows);

    assert!(
        row.contains("other"),
        "the row names the model `/model` stated: {row:?}"
    );
    assert!(
        !row.contains("deepseek"),
        "the launch's model is gone: {row:?}"
    );
}

#[test]
fn a_restated_connection_is_named_on_the_status_row() {
    let mut model = session();
    update(&mut model, restated("deepseek-v4.1-flash", "shop"));

    let rows = painted(&model, &launch(), 60, 24);
    let row = status(&rows);

    assert!(
        row.contains("shop"),
        "the row names the database `/connect` stated: {row:?}"
    );
    assert!(
        !row.contains("demo"),
        "the launch's database is gone: {row:?}"
    );
}

#[test]
fn the_port_restates_the_facts_as_the_message_the_update_owns() {
    let (msgs, mut queue) = tokio::sync::mpsc::unbounded_channel();
    let front = Forward::new(msgs);

    front.restated(String::from("other"), Some(String::from("shop")));

    match queue
        .try_recv()
        .expect("the restated facts are on the queue")
    {
        Msg::Restated { model, database } => {
            assert_eq!(model, "other", "the model the session states");
            assert_eq!(database.as_deref(), Some("shop"), "and its database");
        }
        other => panic!("the queue carried something else: {other:?}"),
    }
}
