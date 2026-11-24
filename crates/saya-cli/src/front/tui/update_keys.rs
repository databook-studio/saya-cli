//! The keys the surface answers itself, carried out: `c` copies the focused
//! entry's source, Shift+Tab walks the mode cycle, and Enter on the mode picker
//! states the mode it chose.
//!
//! One module per family of presses, the shape [`super::update_submit`],
//! [`super::update_answer`] and [`super::changes_press`] keep: `update.rs` reads
//! the press and hands it on, so the decision tree there stays a router.
//!
//! Nothing here touches a terminal or the engine: what leaves the surface is a
//! [`Cmd`] the runtime performs afterwards.

use super::focus_text;
use super::model::Model;
use super::msg::Cmd;
use super::view::ModePicker;
use crate::interactive::session::permissions::mode::next_in_cycle;

/// `c`: copy the focused entry's source text, and say what was copied (`02`
/// §4). An entry with no text of its own — the activity group, whose words are
/// its steps' — copies nothing and claims nothing.
pub(super) fn copy(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let Some((word, text)) = model.focused_entry().and_then(focus_text::plain) else {
        return;
    };
    if text.is_empty() {
        return;
    }
    model.say(focus_text::toast(word, &text));
    cmds.push(Cmd::Copy(text));
}

/// Shift+Tab: the mode cycle's next member, skipping a withheld `bypass`. The
/// cycle is `02` §8's four interactive modes — `read-only` and `never` are
/// reachable only by naming them — and only a change is said (J6).
pub(super) fn cycle(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let next = next_in_cycle(model.mode(), model.allow_bypass);
    model.set_mode(next);
    model.note_mode(next);
    cmds.push(Cmd::SwitchMode(next));
}

/// Enter on the mode picker: the selected row's mode is stated, once, and
/// exactly a change is said where it happened (J6).
pub(super) fn choose(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let Some(chosen) = model.mode_picker().map(ModePicker::chosen) else {
        return;
    };
    model.close_overlay();
    if chosen == model.mode() {
        return;
    }
    model.set_mode(chosen);
    model.note_mode(chosen);
    cmds.push(Cmd::SetMode(chosen));
}
