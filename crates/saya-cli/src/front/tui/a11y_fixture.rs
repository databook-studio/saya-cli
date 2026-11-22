//! The accessibility pass's samples: one per pane the crate renders, each a
//! whole pane's words, so a test can compare the colour-off reading with the
//! coloured one without a terminal.
//!
//! The samples are built here rather than borrowed from each pane's own test
//! module: those live under `view` and are private to it, and this pass is about
//! the panes as a set. Every sample states the design's own case (`02` §2-§11),
//! so a pane reworded into agreeing with itself still reads.

use std::time::Duration;

use ratatui::text::Line;
use saya_agent::PermissionMode;
use saya_types::QueryResult;

use super::model::Model;
use super::theme::{Charset, Scheme};
use super::view::jobs_fixture;
use super::view::{
    ActivityState, Context, Database, Drawer, EmptyFacts, ModePicker, Palette, Picker, SessionRow,
    SessionState, StatusFacts, TranscriptFacts, cards::approval, empty_rows, help_rows, jobs_rows,
    mode_picker_rows, palette_rows, result_rows, rows as composer_rows, sessions_rows, status_line,
    transcript_rows,
};
use crate::front::{
    Activity, Answer, Clarification, Command, CommandState, DiffLine, Edit, Entry, Error, Helper,
    HelperState, Notice, ResultPreview, Step, StepState, Stop, TurnView, UserMessage,
};

/// One pane of the surface the crate renders, by the name a failure mentions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Status,
    Transcript,
    Composer,
    Card,
    Palette,
    Help,
    Sessions,
    ModePicker,
    Jobs,
    Result,
    Empty,
    Blocks,
}

/// Every pane of `02` §2-§11 the crate paints.
pub(crate) const ALL: [Pane; 12] = [
    Pane::Status,
    Pane::Transcript,
    Pane::Composer,
    Pane::Card,
    Pane::Palette,
    Pane::Help,
    Pane::Sessions,
    Pane::ModePicker,
    Pane::Jobs,
    Pane::Result,
    Pane::Empty,
    Pane::Blocks,
];

impl Pane {
    /// The pane's name, as a failure states it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Status => "status line",
            Self::Transcript => "transcript",
            Self::Composer => "composer",
            Self::Card => "docked card",
            Self::Palette => "command palette",
            Self::Help => "help sheet",
            Self::Sessions => "sessions picker",
            Self::ModePicker => "mode picker",
            Self::Jobs => "jobs drawer",
            Self::Result => "result table",
            Self::Empty => "empty state",
            Self::Blocks => "transcript blocks",
        }
    }
}

/// The pane's whole text, row by row, in `scheme` and `charset`.
pub(crate) fn render(pane: Pane, scheme: Scheme, charset: Charset) -> Vec<String> {
    let facts = transcript_facts(scheme, charset);
    match pane {
        Pane::Status => lines(&[status_line(&status_facts(scheme, charset), 100)]),
        Pane::Transcript => lines(&transcript_rows(&[conversation()], &facts, 100, 40)),
        Pane::Composer => row_texts(&composer_rows(&draft(), 100, charset)),
        Pane::Card => card_rows(),
        Pane::Palette => row_texts(&palette_rows(
            &Palette {
                query: String::from("exp"),
                charset,
                ..Palette::default()
            },
            100,
            20,
        )),
        Pane::Help => row_texts(&help_rows(
            Context {
                card: true,
                grant: true,
                charset,
                ..Context::default()
            },
            100,
            20,
        )),
        Pane::Sessions => row_texts(&sessions_rows(&picker(charset), 100, 20)),
        Pane::ModePicker => row_texts(&mode_picker_rows(
            &ModePicker::on(PermissionMode::Ask),
            100,
            10,
            charset,
        )),
        Pane::Jobs => row_texts(&jobs_rows(&drawer(charset), 100, 6, charset)),
        Pane::Result => lines(&result_rows(&preview(), &facts, 100)),
        Pane::Empty => lines(&empty_rows(&empty_facts(scheme, charset), 100, 20)),
        Pane::Blocks => lines(&transcript_rows(&[blocks()], &facts, 100, 40)),
    }
}

/// The status line's facts: every segment on, in bypass so the chip's word is
/// in the sample (`02` §4).
fn status_facts(scheme: Scheme, charset: Charset) -> StatusFacts {
    StatusFacts {
        mode: PermissionMode::Bypass,
        databases: vec![String::from("demo")],
        model: String::from("deepseek-v4.1-flash"),
        workspace: Some(String::from("~/shop")),
        host_unsandboxed: true,
        sharing: true,
        deny: vec![String::from("curl")],
        context_percent: Some(23),
        activity: ActivityState::Working { seconds: 6 },
        scheme,
        charset,
        ..StatusFacts::default()
    }
}

fn transcript_facts(scheme: Scheme, charset: Charset) -> TranscriptFacts {
    TranscriptFacts {
        elapsed: vec![Duration::from_secs(14)],
        workspace: Some(String::from("~/shop")),
        scheme,
        charset,
        ..TranscriptFacts::default()
    }
}

/// A model with a draft typed, for the composer's box.
fn draft() -> Model {
    let mut model = Model::default();
    model.database = Some(String::from("demo"));
    model.insert("show me the orders");
    model
}

/// The docked card's words: the shell call of `02` §5, its reason and keys.
fn card_rows() -> Vec<String> {
    let card = approval(
        &super::view::cards_fixture::shell_question(),
        PermissionMode::Ask,
        24,
    );
    let mut rows = vec![card.title().to_owned()];
    rows.extend(card.shown(false).iter().cloned());
    if let Some(because) = card.because() {
        rows.push(because.to_owned());
    }
    rows.push(card.keys().to_owned());
    rows
}

/// The picker over `02` §10's sessions: one of each resume state.
fn picker(charset: Charset) -> Picker {
    let mut picker = Picker::new(vec![
        session("s-1", "orders by month", SessionState::Open),
        session("s-2", "pricing tests", SessionState::Finished),
        session("s-3", "old record", SessionState::Record),
    ]);
    picker.charset = charset;
    picker
}

fn session(id: &str, title: &str, state: SessionState) -> SessionRow {
    SessionRow {
        id: id.to_owned(),
        title: title.to_owned(),
        database: Some(String::from("demo")),
        turns: 3,
        age: String::from("2h"),
        state,
        read_only: state.read_only(),
    }
}

/// `02` §8's three jobs, in the drawer.
fn drawer(charset: Charset) -> Drawer {
    let mut drawer = Drawer::new(jobs_fixture::jobs());
    drawer.charset = charset;
    drawer
}

/// One successful query's preview (`02` §3.3).
fn preview() -> ResultPreview {
    let sql = String::from("SELECT COUNT(*) AS orders_since_december FROM orders");
    ResultPreview {
        connection: String::from("demo"),
        sql: sql.clone(),
        result: QueryResult {
            columns: vec![String::from("orders_since_december")],
            rows: vec![serde_json::json!([110])],
            row_count: 1,
            truncated: false,
            executed_sql: sql,
        },
        refusal: None,
        evidence: None,
    }
}

fn empty_facts(scheme: Scheme, charset: Charset) -> EmptyFacts {
    EmptyFacts {
        provider: true,
        database: Some(Database {
            name: String::from("demo"),
            dialect: String::from("sqlite"),
            tables: 12,
        }),
        workspace: Some(String::from("~/shop")),
        starter_questions: vec![String::from("What are the top channels?")],
        scheme,
        charset,
    }
}

/// A conversation with the message, the work, the answer and the result.
fn conversation() -> TurnView {
    let mut turn = TurnView::default();
    turn.push(Entry::UserMessage(UserMessage {
        text: String::from("how many orders did we get since December?"),
    }));
    turn.push(Entry::Activity(Activity {
        steps: vec![Step {
            tool: String::from("bounded_sql_query"),
            arguments: serde_json::json!({"connection": "demo", "sql": "SELECT 1"}),
            state: StepState::Done {
                summary: String::from("1 row"),
            },
        }],
        folded: true,
        failed: false,
    }));
    turn.push(Entry::Answer(Answer {
        text: String::from("110 orders since December."),
    }));
    turn.push(Entry::ResultPreview(preview()));
    turn
}

/// One turn holding every block of `02` §3.4-§3.10.
fn blocks() -> TurnView {
    let mut turn = TurnView::default();
    turn.push(Entry::Edit(Edit {
        path: String::from("pricing.py"),
        lines: vec![
            DiffLine::context(14, "    discount = total * percent_off // 100"),
            DiffLine::removed(15, "    return total - discount - discount"),
            DiffLine::added(15, "    return total - discount"),
        ],
    }));
    turn.push(Entry::Command(Command {
        command: String::from("python3 -m unittest"),
        state: CommandState::Finished {
            elapsed: Duration::from_millis(400),
        },
    }));
    turn.push(Entry::Helper(Helper {
        id: String::from("explore-1"),
        question: String::from("find where pricing is tested"),
        steps: 3,
        state: HelperState::Done,
    }));
    turn.push(Entry::Error(Error {
        message: String::from("Provider error — rate limited (429)."),
    }));
    turn.push(Entry::Stop(Stop::Paused {
        message: String::from("no progress in 8 turns."),
    }));
    turn.push(Entry::Stop(Stop::Cancelled));
    turn.push(Entry::Question(Clarification {
        question: String::from("Which revenue do you mean?"),
        options: vec![String::from("Gross order value")],
    }));
    turn.push(Entry::Notice(Notice {
        message: String::from("mode: accept-edits"),
    }));
    turn
}

/// Painted rows as their text.
fn row_texts(rows: &[super::view::Row]) -> Vec<String> {
    rows.iter().map(super::view::Row::text).collect()
}

/// Painted lines as their text, styles dropped.
pub(crate) fn lines(rows: &[Line<'_>]) -> Vec<String> {
    rows.iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

/// One pane's whole text as one string, for a `contains` check.
pub(crate) fn words(pane: Pane, scheme: Scheme, charset: Charset) -> String {
    render(pane, scheme, charset).join("\n")
}
