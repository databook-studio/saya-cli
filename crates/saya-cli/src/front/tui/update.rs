//! The pure update: one message in, the new model and the side effects out.
//!
//! Nothing here touches a terminal, a clock or the engine: every decision a
//! session makes about its own state is this function, and every decision that
//! leaves the surface is a [`Cmd`] the runtime performs afterwards. That split
//! is what makes sending, queueing, cancelling and answering testable without
//! a screen.
//!
//! Two rules shape the rest. A turn in flight takes the operator's next
//! message *behind* it rather than replacing it — the queue is one deep, and
//! the line it displaces is said out loud instead of vanishing. And a stop is
//! never ambiguous: Esc over a running turn stops the turn, while Esc over an
//! open card denies that card, because the card is what the operator is
//! looking at.

use ratatui::crossterm::event::KeyEvent;

use super::direct;
use super::focus_text;
use super::history_keys;
use super::jobs;
use super::keys::{Intent, intent};
use super::model::Model;
use super::msg::{Cmd, Msg};
use super::overlay_press;
use super::update_submit::{send, submit};
use super::view::ModePicker;
use crate::agent::tools::consent::Answer;
use crate::chat::ChatEvent;
use crate::front::apply;
use crate::interactive::session::permissions::mode::next_in_cycle;

/// Folds one message into the model, and answers what the runtime must do.
pub(crate) fn update(model: &mut Model, msg: Msg) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    match msg {
        Msg::Key(key) => press(model, key, &mut cmds),
        // A paste lands as text and nothing else: a `\r` inside a pasted block
        // must never send it.
        Msg::Paste(text) => model.insert(&text),
        // The panes lay themselves out from the frame they are handed, so a
        // new size changes nothing the model holds.
        Msg::Resize { .. } => {}
        Msg::Chat(event) => session(model, event, &mut cmds),
        Msg::Worker(worker) => {
            // A direct command's answer is filed where it belongs — the rows as
            // a result entry, a command's sentence as a notice or an error — and
            // what is left is the worker's own word about itself (`02` §4).
            if let Some(outcome) = worker.direct {
                direct::land(model, outcome);
            }
            // A job's word is the run's job list, or one call it puts to the
            // operator: the call takes the same card slot as the session's.
            if let Some(report) = worker.job {
                model.note_jobs(report);
            }
            if !worker.message.is_empty() {
                model.say(worker.message);
            }
        }
        // The spinner reads this tick, and so does the toast's clock: four
        // seconds of ticks spend it (`02` §4).
        Msg::Tick => model.tick_toast(),
        // Leaving is the loop's decision, not the model's: the runtime ends on
        // this message and the model state it leaves behind is never painted.
        Msg::Quit => {}
    }
    cmds
}

/// One key press, read through the keymap and carried out.
fn press(model: &mut Model, key: KeyEvent, cmds: &mut Vec<Cmd>) {
    match intent(model, key) {
        Some(Intent::Insert(character)) => model.push(character),
        Some(Intent::Newline) => model.push('\n'),
        Some(Intent::Backspace) => {
            model.backspace();
        }
        Some(Intent::Submit) => submit(model, cmds),
        // The queue is one deep, and Ctrl+G is how the operator takes the held
        // line back: it is dropped, never sent (`02` §7).
        Some(Intent::DropQueued) => model.drop_queued(),
        // The history's own keys: a recall fills the line, a search reads the
        // ring — neither sends anything (`02` §7).
        Some(Intent::History(action)) => history_keys::press(model, action),
        Some(Intent::CancelTurn) => cmds.push(Cmd::CancelTurn),
        Some(Intent::AnswerCard(given)) => answer_card(model, given, cmds),
        Some(Intent::ViewCard) => model.toggle_card_full(),
        Some(Intent::ScrollCard(rows)) => model.scroll_card(rows),
        Some(Intent::CycleMode) => cycle(model, cmds),
        Some(Intent::ModeUp) => {
            if let Some(picker) = model.mode_picker_mut() {
                picker.up();
            }
        }
        Some(Intent::ModeDown) => {
            if let Some(picker) = model.mode_picker_mut() {
                picker.down();
            }
        }
        Some(Intent::ModeSet) => choose(model, cmds),
        Some(Intent::ModeCancel) => model.close_overlay(),
        // The panes that own the screen (`02` §11): opening one, walking it, and
        // running the command the palette chooses.
        Some(
            intent @ (Intent::OpenPalette
            | Intent::OpenHelp
            | Intent::PaletteUp
            | Intent::PaletteDown
            | Intent::PaletteInsert(_)
            | Intent::PaletteBackspace
            | Intent::PaletteRun
            | Intent::PaletteClose
            | Intent::HelpClose),
        ) => overlay_press::press(model, intent, cmds),
        Some(Intent::FocusUp) => model.move_focus(false),
        Some(Intent::FocusDown) => model.move_focus(true),
        Some(Intent::FocusExpand) => model.fold_focus(None),
        Some(Intent::FocusFold) => model.fold_focus(Some(true)),
        Some(Intent::FocusCopy) => copy(model, cmds),
        Some(Intent::FocusInspector) => model.toggle_inspector(),
        Some(Intent::FocusLeave) => model.clear_focus(),
        Some(Intent::TrustAccept) => {
            model.close_overlay();
            cmds.push(Cmd::Trust);
        }
        Some(Intent::TrustDecline) => model.close_overlay(),
        Some(Intent::OpenJobs) => model.open_jobs(),
        // The drawer's own keys: walking it, opening a job's call on the card,
        // asking before a cancel, and closing it (`02` §8).
        Some(
            intent @ (Intent::JobsUp
            | Intent::JobsDown
            | Intent::JobsOpen
            | Intent::JobsApprove
            | Intent::JobsCancelAsk
            | Intent::JobsCancelYes
            | Intent::JobsCancelNo
            | Intent::JobsClose),
        ) => jobs::press(model, intent, cmds),
        Some(Intent::Refused(reason)) => model.say(reason),
        None => {}
    }
}

/// `c`: copy the focused entry's source text, and say what was copied (`02`
/// §4). An entry with no text of its own — the activity group, whose words are
/// its steps' — copies nothing and claims nothing.
fn copy(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let Some((word, text)) = model.focused_entry().and_then(focus_text::plain) else {
        return;
    };
    if text.is_empty() {
        return;
    }
    model.say(focus_text::toast(word, &text));
    cmds.push(Cmd::Copy(text));
}

/// One answer to the open card: the call is answered, and the turn that asked
/// keeps going. An approval is the operator's consent, so the answer is
/// carried out exactly as it was given — once, for the session, or never.
fn answer_card(model: &mut Model, given: Answer, cmds: &mut Vec<Cmd>) {
    if let Some(card) = model.take_card() {
        cmds.push(Cmd::Answer(card.id, given));
        // One card at a time: the job that owed this call no longer waits, and
        // the ask behind it takes the slot it left (`02` §5, T9/A5).
        model.answered(card.id);
    }
}

/// Shift+Tab: the mode cycle's next member, skipping a withheld `bypass`. The
/// cycle is `02` §8's four interactive modes — `read-only` and `never` are
/// reachable only by naming them — and only a change is said (J6).
fn cycle(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let next = next_in_cycle(model.mode(), model.allow_bypass);
    model.set_mode(next);
    model.note_mode(next);
    cmds.push(Cmd::SwitchMode(next));
}

/// Enter on the mode picker: the selected row's mode is stated, once, and
/// exactly a change is said where it happened (J6).
fn choose(model: &mut Model, cmds: &mut Vec<Cmd>) {
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

/// One session event: folded into the transcript and, when it ends a turn,
/// answered by whatever the operator queued behind it.
fn session(model: &mut Model, event: ChatEvent, cmds: &mut Vec<Cmd>) {
    let ended = matches!(&event, ChatEvent::TurnEnded(_));
    if let ChatEvent::Question(question) = &event {
        // The session's ask and a job's ask enter the same slot by the same
        // rule: on the card when it is free, behind it otherwise.
        model.ask(question.clone());
    }
    apply(&mut model.transcript, &event);
    if ended {
        model.end_turn();
        if let Some(text) = model.take_queued() {
            send(model, text, cmds);
        }
    }
}
