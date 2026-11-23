//! Replacing the last turn (UI8): the resend a restored composer line asks for
//! rewinds the previous turn's file changes through the run's own checkpoints
//! before the edited message runs, and skips — naming — a file another run
//! changed since.
//!
//! [`rewind_last`] is the exact rewind half the line's replace runs; the
//! checkpoints below are fakes over a scratch folder, because a real run's
//! checkpoints are the coding job's own (covered by the `/undo` tests).

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use ratatui::crossterm::event::KeyCode;
use saya_store::RedactedTurn;
use tokio::sync::mpsc;

use super::msg::Msg;
use super::run::Runtime;
use super::run_fixture::{FakeScreen, key, typed};
use super::run_fixture_port::{Call, FakeClock, FakePort};
use super::update::update;
use super::update_fixture::{enter, escape};
use crate::chat::{ChatEvent, ChatOutcome, Checkpoint, Checkpoints, Restored, Transcript};
use crate::front::tui::direct::DirectOutcome;
use crate::interactive::session::line_chat::Line;

/// A run of two checkpoints over one scratch folder: restoring the older one
/// writes it back, unless another run changed the file since.
struct FakeRun {
    folder: PathBuf,
    changed_elsewhere: Vec<String>,
    restored: Mutex<Vec<u32>>,
}

impl FakeRun {
    fn new(folder: PathBuf, changed_elsewhere: &[&str]) -> Self {
        Self {
            folder,
            changed_elsewhere: changed_elsewhere
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
            restored: Mutex::new(Vec::new()),
        }
    }

    fn restored(&self) -> Vec<u32> {
        self.restored.lock().unwrap().clone()
    }
}

#[async_trait]
impl Checkpoints for FakeRun {
    async fn checkpoints(&self) -> Result<Vec<Checkpoint>, String> {
        Ok(vec![
            Checkpoint {
                turn: 1,
                files: 1,
                first_words: Some("the first note".to_owned()),
            },
            Checkpoint {
                turn: 2,
                files: 1,
                first_words: Some("the second note".to_owned()),
            },
        ])
    }

    async fn restore(&self, turn: u32) -> Result<Restored, String> {
        self.restored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(turn);
        if !self.changed_elsewhere.is_empty() {
            return Ok(Restored {
                restored: Vec::new(),
                changed_elsewhere: self.changed_elsewhere.clone(),
            });
        }
        fs::write(self.folder.join("notes.txt"), "one\n").unwrap();
        Ok(Restored {
            restored: vec!["notes.txt".to_owned()],
            changed_elsewhere: Vec::new(),
        })
    }
}

/// A scratch folder removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("tmp-ui8-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Two turns in the visible conversation: the second one is being replaced.
fn transcript() -> Transcript {
    let turn = |prompt: &str| RedactedTurn {
        user: prompt.to_owned(),
        assistant: String::from("an answer"),
        database_derived: false,
        tools: Vec::new(),
    };
    Transcript::from_record(saya_store::RedactedSession {
        turns: vec![turn("the first note"), turn("the second note")],
        ..saya_store::RedactedSession::default()
    })
}

/// The one notice the front was handed, drained.
async fn notice(rx: &mut mpsc::UnboundedReceiver<Msg>) -> String {
    match rx.recv().await.expect("the front said something") {
        Msg::Worker(worker) => match worker.direct {
            Some(DirectOutcome::Notice(line)) => line,
            other => panic!("expected a notice, got {other:?}"),
        },
        other => panic!("expected a worker message, got {other:?}"),
    }
}

#[tokio::test]
async fn a_resend_rewinds_the_previous_turns_edits_and_drops_the_turn() {
    let scratch = Scratch::new("rewind");
    fs::write(scratch.0.join("notes.txt"), "two\n").unwrap();
    let run = FakeRun::new(scratch.0.clone(), &[]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let front = super::run_front::Forward::new(tx);
    let mut transcript = transcript();

    Line::rewind_last(&mut transcript, Some(&run), &front).await;

    assert_eq!(
        fs::read_to_string(scratch.0.join("notes.txt")).unwrap(),
        "one\n",
        "the replaced turn's file change is undone before the edited message runs"
    );
    assert_eq!(
        transcript.turns().len(),
        1,
        "the replaced turn leaves the conversation the next run replays"
    );
    assert_eq!(
        run.restored(),
        vec![1],
        "the checkpoint before the last turn"
    );
    let line = notice(&mut rx).await;
    assert!(
        line.contains("undid the last turn's edits") && line.contains("notes.txt"),
        "the confirmation line says what was undone: {line}"
    );
}

#[tokio::test]
async fn a_file_changed_elsewhere_is_skipped_and_named() {
    let scratch = Scratch::new("skip");
    fs::write(scratch.0.join("notes.txt"), "another run\n").unwrap();
    let run = FakeRun::new(scratch.0.clone(), &["notes.txt"]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let front = super::run_front::Forward::new(tx);
    let mut transcript = transcript();

    Line::rewind_last(&mut transcript, Some(&run), &front).await;

    assert_eq!(
        fs::read_to_string(scratch.0.join("notes.txt")).unwrap(),
        "another run\n",
        "the other run's bytes stand: a rewind never overwrites another run's write"
    );
    let line = notice(&mut rx).await;
    assert!(
        line.contains("changed since by another run: notes.txt"),
        "the skipped file is named as another run's change: {line}"
    );
}

/// The resend the composer asks for reaches the line's own replace operation,
/// never a second send.
#[tokio::test]
async fn the_resend_reaches_the_lines_own_replace_operation() {
    let mut model = typed("count the orders");
    update(&mut model, enter());
    update(
        &mut model,
        Msg::Chat(ChatEvent::TurnEnded(ChatOutcome::Answered)),
    );
    update(&mut model, escape());
    update(&mut model, escape());

    let (sender, queue) = mpsc::unbounded_channel();
    let port = Arc::new(FakePort::new());
    let mut runtime = Runtime::new(
        model,
        queue,
        FakeScreen::new(100, 30),
        FakeClock::new(),
        Arc::clone(&port),
    );
    sender.send(key(KeyCode::Enter)).expect("the loop reads");
    runtime.step().await.expect("one turn of the loop");

    assert_eq!(
        port.calls(),
        vec![Call::Replace("count the orders".to_owned())],
        "the edited message is handed to the line's replace, not a plain send"
    );
}
