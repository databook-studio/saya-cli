//! The drawer through the runtime: `Ctrl+J` paints it where `02` §8 says it
//! goes — the transcript's right-hand side at 100 columns, a bottom sheet at 80
//! — and `x` answered on the fake port is one cancel of the row it named.

use std::sync::Arc;
use std::sync::{Mutex, PoisonError};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use super::jobs_fixture::listed;
use super::model::Model;
use super::msg::Msg;
use super::run::{FRAME, Runtime};
use super::run_fixture::FakeScreen;
use super::run_fixture_port::{Call, FakeClock, FakePort};

/// The last frame the fake screen painted, one row per line and without the
/// backend's quoting.
fn last(recorded: &Arc<Mutex<super::run_fixture::Recorded>>) -> String {
    recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .last_frame()
        .lines()
        .map(|row| row.trim_matches('"'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A model holding the run's three jobs.
fn model() -> Model {
    let mut model = Model::default();
    super::update::update(&mut model, listed());
    model
}

/// `Ctrl+J`.
fn ctrl_j() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL))
}

#[tokio::test]
async fn at_a_hundred_columns_the_drawer_takes_the_transcripts_side() {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(100, 30);
    let recorded = screen.recorded();
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let mut runtime = Runtime::new(model(), queue, screen, clock.clone(), Arc::clone(&port));

    sender.send(ctrl_j()).expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");

    let painted = last(&recorded);
    assert!(painted.contains("╭ Background jobs"), "{painted}");
    assert!(painted.contains("running 3m · 2 helpers"), "{painted}");
    assert!(
        port.calls().is_empty(),
        "opening the drawer tells the session nothing: {:?}",
        port.calls()
    );
    // The drawer shares its row with the transcript's own first row, so it sits
    // beside the transcript rather than across it.
    let top = painted
        .lines()
        .find(|row| row.contains("╭ Background jobs"))
        .expect("the drawer's top border is painted");
    assert!(
        top.find("╭ Background jobs").unwrap_or_default() > 40,
        "the drawer is on the right: {top:?}"
    );
}

#[tokio::test]
async fn at_eighty_columns_the_drawer_is_a_bottom_sheet() {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(80, 24);
    let recorded = screen.recorded();
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let mut runtime = Runtime::new(model(), queue, screen, clock.clone(), port);

    sender.send(ctrl_j()).expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");

    let painted = last(&recorded);
    let rows: Vec<&str> = painted.lines().collect();
    let top = rows
        .iter()
        .position(|row| row.contains("╭ Background jobs"))
        .expect("the sheet is painted");
    assert!(
        rows[top].trim_start().starts_with('╭'),
        "the sheet spans the width: {:?}",
        rows[top]
    );
    assert!(
        rows.iter().any(|row| row.contains("╭ Message")),
        "the composer keeps the rows below it: {painted}"
    );
}

#[tokio::test]
async fn x_answered_on_the_port_cancels_the_row_it_named() {
    let (sender, queue) = mpsc::unbounded_channel();
    let screen = FakeScreen::new(100, 30);
    let clock = FakeClock::new();
    let port = Arc::new(FakePort::new());
    let mut runtime = Runtime::new(model(), queue, screen, clock.clone(), Arc::clone(&port));

    for message in [
        ctrl_j(),
        Msg::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
    ] {
        sender.send(message).expect("the loop reads");
        clock.advance(FRAME);
        runtime.step().await.expect("one turn of the loop");
    }
    assert!(port.calls().is_empty(), "x alone cancels nothing");

    sender
        .send(Msg::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )))
        .expect("the loop reads");
    clock.advance(FRAME);
    runtime.step().await.expect("one turn of the loop");

    assert_eq!(
        port.calls(),
        vec![Call::CancelJob(String::from("fix-flaky"))],
        "one cancel, of the row the ask named"
    );
}
