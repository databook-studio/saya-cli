//! Test support for UI5's two surfaces: a turn that wrote two files, and the
//! model that holds it — the state the `Changed:` line, the changes pane and the
//! key sheet are read in.

use super::model::Model;
use crate::front::view::{Activity, Entry, Step, StepState};

/// A file call that landed, as the turn's activity block states it.
pub(crate) fn edited(path: &str, old_text: &str, new_text: &str) -> Step {
    Step {
        tool: String::from("edit"),
        arguments: serde_json::json!({
            "path": path,
            "old_text": old_text,
            "new_text": new_text,
        }),
        state: StepState::Done {
            summary: String::from("workspace file edited"),
        },
    }
}

/// A whole-file write that landed.
pub(crate) fn written(path: &str, content: &str) -> Step {
    Step {
        tool: String::from("write"),
        arguments: serde_json::json!({"path": path, "content": content}),
        state: StepState::Done {
            summary: String::from("workspace file written"),
        },
    }
}

/// A turn whose activity block holds `steps`.
fn turn_entries(steps: Vec<Step>) -> Vec<Entry> {
    vec![Entry::Activity(Activity {
        steps,
        ended: true,
        ..Activity::default()
    })]
}

/// The two files UI5's screens are read from: a one-line replace and a
/// three-line write.
pub(crate) fn steps() -> Vec<Step> {
    vec![
        edited(
            "src/calc.py",
            "return total - discount - discount",
            "return total - discount",
        ),
        written("notes/todo.md", "one\ntwo\nthree\n"),
    ]
}

/// A model whose last turn wrote those two files: what the operator sees after a
/// turn that changed something.
pub(crate) fn model_with_edits() -> Model {
    let mut model = Model::default();
    model.begin_turn(String::from("tighten the pricing tests"));
    let turn = model.transcript.current();
    for entry in turn_entries(steps()) {
        turn.push(entry);
    }
    model
}

/// The same steps, on a surface in accept-edits mode: no card is put to the
/// operator, and the turn still changed the files.
pub(crate) fn accept_edits_model() -> Model {
    let mut model = model_with_edits();
    model.set_mode(saya_agent::PermissionMode::AcceptEdits);
    model
}

/// A call the arguments of which state no change: a read of the same file.
pub(crate) fn read_step() -> Step {
    Step {
        tool: String::from("read"),
        arguments: serde_json::json!({"path": "src/calc.py"}),
        state: StepState::Done {
            summary: String::from("read"),
        },
    }
}
