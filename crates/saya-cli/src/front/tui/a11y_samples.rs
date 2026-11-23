//! The facts the accessibility pass's pane samples are built from.
//!
//! Kept apart from [`super::a11y_fixture`] so each file stays one concern: these
//! are the design's own cases (`02` §2-§11) as values, and the fixture above
//! turns them into a pane's words.

use std::time::Duration;

use saya_agent::PermissionMode;
use saya_types::QueryResult;

use super::model::Model;
use super::theme::{Charset, Scheme};
use super::view::jobs_fixture;
use super::view::{
    ActivityState, Context, Database, Drawer, EmptyFacts, ModePicker, Palette, Picker, SessionRow,
    SessionState, StatusFacts, TranscriptFacts, cards::approval,
};
use crate::front::{
    Activity, Answer, Clarification, Command, CommandState, DiffLine, Edit, Entry, Error, Helper,
    HelperState, Notice, ResultPreview, Step, StepState, Stop, TurnView, UserMessage,
};

/// The status line's facts: the mode in bypass so the chip's word is in the
/// sample, the model, and the database (`02` §4).
pub(crate) fn status_facts(scheme: Scheme, charset: Charset) -> StatusFacts {
    StatusFacts {
        mode: PermissionMode::Bypass,
        databases: vec![String::from("demo")],
        model: String::from("deepseek-v4.1-flash"),
        sharing: true,
        activity: ActivityState::Working { seconds: 6 },
        meters: super::view::Meters::default(),
        scheme,
        charset,
    }
}

pub(crate) fn transcript_facts(scheme: Scheme, charset: Charset) -> TranscriptFacts {
    TranscriptFacts {
        elapsed: vec![Duration::from_secs(14)],
        workspace: Some(String::from("~/shop")),
        scheme,
        charset,
        ..TranscriptFacts::default()
    }
}

/// A model with a draft typed, for the composer's box.
pub(crate) fn draft() -> Model {
    let mut model = Model {
        database: Some(String::from("demo")),
        ..Model::default()
    };
    model.insert("show me the orders");
    model
}

/// The docked card's words: the shell call of `02` §5, where it runs and the
/// answers it offers.
pub(crate) fn card_rows() -> Vec<String> {
    let card = approval(
        &super::view::cards_fixture::shell_question(),
        Some("~/shop"),
        24,
    );
    let mut rows = vec![card.title().to_owned()];
    if let Some(place) = card.place() {
        rows.push(place.to_owned());
    }
    rows.extend(card.shown_rows(false, 96));
    for choice in card.choices() {
        rows.push(format!("[{}] {}", choice.key, choice.label));
    }
    rows
}

/// The command palette of `02` §11, narrowed by `exp`.
pub(crate) fn palette(charset: Charset) -> Palette {
    Palette {
        query: String::from("exp"),
        charset,
        ..Palette::default()
    }
}

/// The help sheet, as the whole keyboard, in the sample's glyph set.
pub(crate) fn help_context(charset: Charset) -> Context {
    Context { charset }
}

/// The picker over `02` §10's sessions: one of each resume state.
pub(crate) fn picker(charset: Charset) -> Picker {
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

/// `02` §8's mode picker, on the mode in force.
pub(crate) fn mode_picker() -> ModePicker {
    ModePicker::on(PermissionMode::Ask)
}

/// `02` §8's three jobs, in the drawer.
pub(crate) fn drawer(charset: Charset) -> Drawer {
    let mut drawer = Drawer::new(jobs_fixture::jobs());
    drawer.charset = charset;
    drawer
}

/// One successful query's preview (`02` §3.3).
pub(crate) fn preview() -> ResultPreview {
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

/// `02` §9.3: a connected database with its suggested questions.
pub(crate) fn empty_facts(scheme: Scheme, charset: Charset) -> EmptyFacts {
    EmptyFacts {
        provider: true,
        database: Some(Database {
            name: String::from("demo"),
            dialect: String::from("sqlite"),
            tables: 12,
        }),
        workspace: Some(String::from("~/shop")),
        starter_questions: vec![String::from("What are the top channels?")],
        suggestions: Vec::new(),
        scheme,
        charset,
    }
}

/// A conversation with the message, the work, the answer and the result.
pub(crate) fn conversation() -> TurnView {
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
        ended: true,
        folded: false,
        failed: false,
    }));
    turn.push(Entry::Answer(Answer {
        // The answer carries a fenced block: the parity checks then cover the
        // highlight too — the same words with colour off, and in seven bits.
        text: String::from(
            "110 orders since December.\n\n```sql\nSELECT count(*) FROM orders; -- one row\n```",
        ),
    }));
    turn.push(Entry::ResultPreview(preview()));
    turn
}

/// One turn holding every block of `02` §3.4-§3.10.
pub(crate) fn blocks() -> TurnView {
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
