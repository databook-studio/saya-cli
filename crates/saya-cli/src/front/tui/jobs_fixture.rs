//! The messages the drawer's tests fold: the run's job list, one call a job puts
//! to the operator, the chord that opens the drawer, and a model already holding
//! the run's three jobs.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::jobs::JobReport;
use super::model::Model;
use super::msg::{Msg, Worker};
use super::update::update;
use super::view::jobs::{Job, State};
use crate::chat::{ChatEvent, Question};
use crate::front::Entry;

/// The run's three jobs: one per state of `02` §8.
pub(super) fn rows() -> Vec<Job> {
    vec![
        Job {
            id: String::from("fix-flaky"),
            state: State::Running,
            age: String::from("3m"),
            helpers: 2,
            summary: String::from("query demo · 3 rows"),
            call: None,
        },
        Job {
            id: String::from("docs-pass"),
            state: State::Paused,
            summary: String::from("no progress"),
            ..Job::default()
        },
        Job {
            id: String::from("bump-deps"),
            state: State::Finished,
            summary: String::from("4 files changed"),
            ..Job::default()
        },
    ]
}

/// The run's jobs, as the job plumbing lists them.
pub(super) fn listed() -> Msg {
    Msg::Worker(Worker {
        job: Some(JobReport::Listed(rows())),
        ..Worker::default()
    })
}

/// One call a job puts to the operator, under the question's own id.
pub(super) fn asked(job: &str, id: u64) -> Msg {
    Msg::Worker(Worker {
        job: Some(JobReport::Ask {
            job: String::from(job),
            question: Question {
                id,
                caller: format!("{job}: the shell tool"),
                facts: format!("$ run {id}"),
                grant: None,
                ..Default::default()
            },
        }),
        ..Worker::default()
    })
}

/// The call the session's own turn puts to the operator.
pub(super) fn card(id: u64) -> Msg {
    Msg::Chat(ChatEvent::Question(Question {
        id,
        caller: String::from("Saya"),
        facts: String::from("SELECT COUNT(*) FROM orders"),
        grant: Some(String::from("sql:demo")),
        ..Default::default()
    }))
}

/// `Ctrl+J`, as the input thread reads it.
pub(super) fn ctrl_j() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL))
}

/// One press of a bare key.
pub(super) fn press(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// A model holding the run's three jobs, the drawer not open.
pub(super) fn model() -> Model {
    let mut model = Model::default();
    let cmds = update(&mut model, listed());
    assert!(cmds.is_empty(), "listing jobs states nothing: {cmds:?}");
    model
}

/// The transcript's notices, oldest first.
pub(super) fn notices(model: &Model) -> Vec<String> {
    model
        .turns()
        .iter()
        .flat_map(|turn| turn.entries())
        .filter_map(|entry| match entry {
            Entry::Notice(notice) => Some(notice.message.clone()),
            _ => None,
        })
        .collect()
}
