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

use super::changes_press;
use super::direct;
use super::history_keys;
use super::jobs;
use super::keys::{Intent, intent};
use super::model::Model;
use super::msg::{Cmd, Msg};
use super::overlay_press;
use super::update_answer::{answer_at, answer_card};
use super::update_keys;
use super::update_submit::{send, submit};
use crate::chat::ChatEvent;
use crate::front::apply;

/// Folds one message into the model, and answers what the runtime must do.
pub(crate) fn update(model: &mut Model, msg: Msg) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    match msg {
        Msg::Key(key) => press(model, key, &mut cmds),
        // The `@` list's paths, from the walk the surface asked for: they fill
        // the list it asked on, and a list the operator has closed drops them.
        Msg::Mentions(paths) => model.fill_mentions(paths),
        // A paste lands as text and nothing else: a `\r` inside a pasted block
        // must never send it. While a search owns the line, the text narrows the
        // query, exactly as the same characters typed would.
        Msg::Paste(text) => model.paste(&text),
        // The panes lay themselves out from the frame they are handed, so a
        // new size changes nothing the model holds.
        Msg::Resize { .. } => {}
        Msg::Chat(event) => session(model, event, &mut cmds),
        // Whether the window has the operator's focus is the runtime's own
        // fact, read where a long turn ends; it changes nothing in the model.
        Msg::Focus(_) => {}
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
                // A call that takes the card rings the terminal, once: the
                // operator may be looking at another window.
                if model.note_jobs(report) {
                    ring(model, &mut cmds);
                }
            }
            if !worker.message.is_empty() {
                model.say(worker.message);
            }
        }
        // The spinner reads this tick, and so does the toast's clock: four
        // seconds of ticks spend it, and a step in flight turns one frame
        // (`02` §4).
        Msg::Tick => model.tick(),
        // Leaving is the loop's decision, not the model's: the runtime ends on
        // this message and the model state it leaves behind is never painted.
        Msg::Quit => {}
        // The session restated the model and the database it is on: the status
        // row names those from now on, not the launch's.
        Msg::Restated {
            model: name,
            database,
        } => model.restated(name, database),
        // The settled turn said what it measured: the row's meter segment is
        // those figures from the next frame on.
        Msg::Meters(meters) => model.metered(meters),
    }
    cmds
}

/// One key press, read through the keymap and carried out.
fn press(model: &mut Model, key: KeyEvent, cmds: &mut Vec<Cmd>) {
    // The press is stamped before it is read: a call that opens next keeps the
    // keyboard in the composer when the operator's hands are still on it.
    model.typed_now();
    let read = intent(model, key);
    // The first Esc arms an edit; any other key — including a second key that
    // is not the second Esc — puts the arm down again.
    if !matches!(read, Some(Intent::EditLast)) {
        model.disarm_edit();
    }
    match read {
        Some(Intent::Insert(character)) => {
            model.clear_choice();
            model.push(character);
            // Typing `@`, or the path after it, keeps the list in step: it opens
            // on the mention's first character, narrows as the path is typed,
            // and goes away when a space ends the mention.
            if model.open_mention() {
                cmds.push(Cmd::Mentions);
            }
            if crate::chat::mentions::query(model.composer()).is_none() {
                model.close_mention();
            }
        }
        Some(Intent::Newline) => {
            model.clear_choice();
            model.push('\n');
            model.close_mention();
        }
        Some(Intent::Backspace) => {
            model.clear_choice();
            model.backspace();
            if crate::chat::mentions::query(model.composer()).is_none() {
                model.close_mention();
            }
        }
        Some(Intent::MentionAccept) => {
            model.accept_mention();
        }
        Some(Intent::MentionClose) => model.close_mention(),
        Some(Intent::MentionMove(steps)) => model.move_mention(steps),
        Some(Intent::Submit) => submit(model, cmds),
        // The queue is one deep, and Ctrl+G is how the operator takes the held
        // line back: it is dropped, never sent (`02` §7).
        Some(Intent::DropQueued) => model.drop_queued(),
        // The history's own keys: a recall fills the line, a search reads the
        // ring — neither sends anything (`02` §7).
        Some(Intent::History(action)) => history_keys::press(model, action),
        Some(Intent::CancelTurn) => cmds.push(Cmd::CancelTurn),
        // Esc on an empty composer, with no turn running: the first arms the
        // edit, the second brings the last message back.
        Some(Intent::EditLast) => edit_last(model),
        Some(Intent::AnswerCard(given)) => answer_card(model, given, cmds),
        Some(Intent::ViewCard) => model.toggle_card_full(),
        Some(Intent::ScrollCard(rows)) => model.scroll_card(rows),
        Some(Intent::FocusChoice(step)) => model.move_choice(step),
        Some(Intent::ToggleSeat) => model.toggle_seat(),
        Some(Intent::AnswerFocused) => {
            // Enter answers only what an arrow press has pointed at, so a draft
            // typed during a call can never be sent as a consent.
            if let Some(index) = model.card_choice() {
                let answer = answer_at(model, index);
                if let Some(answer) = answer {
                    answer_card(model, answer, cmds);
                }
            }
        }
        Some(Intent::CycleMode) => update_keys::cycle(model, cmds),
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
        Some(Intent::ModeSet) => update_keys::choose(model, cmds),
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
            | Intent::HelpClose
            | Intent::HelpUp
            | Intent::HelpDown
            | Intent::HelpPageUp
            | Intent::HelpPageDown),
        ) => overlay_press::press(model, intent, cmds),
        // The changes pane (UI5): its keys are `changes_press`'s list, and this
        // hands it every press that list names.
        Some(intent) if changes_press::handles(intent) => changes_press::press(model, intent),
        Some(Intent::FocusUp) => model.move_focus(false),
        Some(Intent::FocusDown) => model.move_focus(true),
        Some(Intent::FocusExpand) => model.fold_focus(None),
        Some(Intent::FocusFold) => model.fold_focus(Some(true)),
        Some(Intent::FocusCopy) => update_keys::copy(model, cmds),
        Some(Intent::FocusInspector) => model.toggle_inspector(),
        Some(Intent::ToggleDetail) => model.toggle_detail(),
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
        // No press, or one no pane above answers: the surface holds it.
        _ => {}
    }
}

/// Puts the terminal's bell on the command list, unless the operator turned
/// notifications off. Every path that takes the card rings through here, so
/// the switch cannot be bypassed by one of them.
fn ring(model: &Model, cmds: &mut Vec<Cmd>) {
    if model.notifications_enabled() {
        cmds.push(Cmd::Bell);
    }
}

/// The first Esc on an empty, settled composer arms an edit; the second puts
/// the last turn's message back on the line, to be sent again.
fn edit_last(model: &mut Model) {
    if model.take_armed() {
        model.restore_last();
    } else {
        model.arm_edit();
    }
}

/// One session event: folded into the transcript and, when it ends a turn,
/// answered by whatever the operator queued behind it.
fn session(model: &mut Model, event: ChatEvent, cmds: &mut Vec<Cmd>) {
    let ended = matches!(&event, ChatEvent::TurnEnded(_));
    if let ChatEvent::Question(question) = &event {
        // The session's ask and a job's ask enter the same slot by the same
        // rule: on the card when it is free, behind it otherwise — and a call
        // that takes the card rings the terminal, once.
        if model.ask(question.clone()) {
            ring(model, cmds);
        }
    }
    apply(&mut model.transcript, &event);
    if ended {
        model.end_turn();
        if let Some(text) = model.take_queued() {
            send(model, text, cmds);
        }
    }
}
