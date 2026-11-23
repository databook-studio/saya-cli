//! The conversation the block cursor's tests walk, and the presses they feed it.
//!
//! Two turns of the view model T5 paints: the older one holds a folded activity
//! group and an answer whose size `02` §4's toast can name; the newer one holds
//! a folded group of its own. The presses are the input thread's own
//! (`KeyEvent`s), so an `Alt+↑` here is the one a terminal sends.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

use super::model::Model;
use super::msg::Msg;
use crate::front::view::{Activity, Answer, Entry, Step, StepState};

/// The older turn's answer: sixty lines of twenty bytes, so the copy's size is
/// `1.2 kB` — the figure `02` §4's toast names.
pub(super) fn answer() -> String {
    "SELECT 1 from order\n".repeat(60)
}

/// One settled step of an activity block.
fn step(tool: &str) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments: json!({"connection": "demo"}),
        state: StepState::Done {
            summary: "12 tables".to_owned(),
        },
    }
}

/// Two turns: the older one's group folded to its summary line and its answer
/// readable, the newer one's group folded too. `failed` marks the older group as
/// the one a step failed in.
pub(super) fn conversation(failed: bool) -> Model {
    let mut model = Model::default();
    model.begin_turn("how many orders did we get since December?".to_owned());
    model.transcript.current().push(Entry::Activity(Activity {
        steps: vec![step("schema_discovery")],
        ended: true,
        folded: true,
        failed,
    }));
    model
        .transcript
        .current()
        .push(Entry::Answer(Answer { text: answer() }));
    model.begin_turn("and by region?".to_owned());
    model.transcript.current().push(Entry::Activity(Activity {
        steps: vec![step("schema_discovery"), step("bounded_sql_query")],
        ended: true,
        folded: true,
        failed: false,
    }));
    model
}

/// One press.
pub(super) fn press(code: KeyCode, modifiers: KeyModifiers) -> Msg {
    Msg::Key(KeyEvent::new(code, modifiers))
}

/// `Alt+↑`: the cursor one entry older.
pub(super) fn alt_up() -> Msg {
    press(KeyCode::Up, KeyModifiers::ALT)
}

/// `Alt+↓`: the cursor one entry newer.
pub(super) fn alt_down() -> Msg {
    press(KeyCode::Down, KeyModifiers::ALT)
}

/// The plain Esc.
pub(super) fn escape() -> Msg {
    press(KeyCode::Esc, KeyModifiers::NONE)
}

/// Whether the turn at `turn` holds a folded activity group.
pub(super) fn folded_of(model: &Model, turn: usize) -> Option<bool> {
    model
        .turns()
        .get(turn)?
        .entries()
        .iter()
        .find_map(|entry| entry.activity().map(|activity| activity.folded))
}
