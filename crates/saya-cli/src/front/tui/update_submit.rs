//! One composer line, and where it goes.
//!
//! Enter on the composer is the surface's one dispatch, and every path into the
//! run goes through it: a line the surface answers itself opens its own pane
//! (`02` §8), a direct command becomes a worker's job (`03` §4.1), a session
//! operation goes to the chat layer's one table (A9), and everything else is a
//! turn — held behind a running one, or sent.
//!
//! It is one function because `02` §11's palette runs a chosen command here
//! too: what the palette does is put `/<name> ` in the composer and submit it,
//! so a command chosen from the list and the same command typed by hand cannot
//! take two different routes.

use super::direct;
use super::jobs;
use super::model::Model;
use super::msg::Cmd;
use crate::slash::param_redact::redact_param_values;
use crate::slash::{SlashCommand, parse_slash_command};

/// What the surface says when a queued line is replaced by a newer one.
const QUEUED_REPLACED: &str = "queued line replaced — the earlier one was dropped";

/// The one slash command the surface answers itself: bare `/mode` opens the
/// picker (`02` §8). K2 gives `/mode <name>` its argument.
const SLASH_MODE: &str = "/mode";

/// The other one: bare `/job` opens the background jobs drawer (`02` §8).
const SLASH_JOB: &str = "/job";

/// The third: bare `/help` opens `02` §11's key sheet. `/help <command>` is
/// still the chat layer's read of the catalogue, so a topic keeps its answer.
const SLASH_HELP: &str = "/help";

/// Enter: send the composer, or hold it behind the turn already running.
pub(super) fn submit(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let text = model.take_composer();
    if text.trim().is_empty() {
        return;
    }
    let line = text.trim();
    // Every submitted line is the ring's, whatever it turns out to be: what the
    // `input_history` file saves and what a later `↑` recalls. The ring redacts
    // a `--param` value on the way in, so the file and the recall show
    // `--param name=…` (`02` §7).
    model.remember(line);
    // The displayed copy of the line — the transcript's echo — carries the same
    // redaction, so a value the ring never persists is never echoed either.
    let shown = redact_param_values(line);
    // A line the surface answers itself: bare `/mode` opens the picker, bare
    // `/job` opens the jobs drawer and bare `/help` opens `02` §11's key sheet,
    // none of them sending anything (`02` §8). All three are read before the
    // slash dispatch, because the crate's parser answers `/job` with its verbs
    // — and bare `/job` has none.
    //
    // A pane never opens over a card that is waiting for an answer: the card is
    // what the operator is looking at, and the one line that acts on it is the
    // job's consent by name (`02` §8).
    if model.card().is_none() {
        if line == SLASH_MODE {
            model.forget_turn();
            model.open_mode_picker();
            return;
        }
        if line == SLASH_JOB {
            model.forget_turn();
            model.open_jobs();
            return;
        }
        if line == SLASH_HELP {
            model.forget_turn();
            model.open_help();
            return;
        }
    }
    // A line the surface answers itself — `/sql`, `/export`, `/chart`,
    // `/investigation` — is one command, never a turn (`03` §4.1). Every other
    // line is a turn's.
    match direct::read(model, &text) {
        direct::Read::Cmd(cmd) => {
            // The line opens the turn its answer lands in: a command's own
            // words are echoed where its rows will be, never only in the
            // composer that has just been cleared.
            model.forget_turn();
            model.begin_turn(shown);
            cmds.push(cmd);
            return;
        }
        direct::Read::Said(words) => {
            // A line the surface answers with words instead of running — a
            // refusal, a pointer — is echoed the same way, redacted.
            model.forget_turn();
            model.begin_turn(shown);
            model.say(words);
            return;
        }
        direct::Read::Turn => {}
    }
    // A command the surface does not answer itself is the session's own
    // operation, read by the chat layer's one table (A9) — never a turn. A job
    // line is the job's own surface instead: `/start` proposes a job whose
    // consent is a card on this surface, the verbs steer the job that runs, and
    // `/approve`/`/deny` answer that card by name (`02` §8). `/mode` stays a
    // turn here: it is read (and dispatched) by the run the surface is composed
    // over.
    match parse_slash_command(line) {
        Ok(Some(SlashCommand::Coding(request))) => {
            model.forget_turn();
            jobs::line(model, request, cmds);
            return;
        }
        Ok(Some(command)) if !matches!(command, SlashCommand::Mode(_)) => {
            model.forget_turn();
            cmds.push(Cmd::Op(command));
            return;
        }
        _ => {}
    }
    if model.is_busy() {
        // The edit was armed before a turn started: a queued line is the next
        // turn's message, not a replacement of the one in flight.
        model.take_replacing();
        if model.queue(text).is_some() {
            model.say(QUEUED_REPLACED);
        }
        return;
    }
    // A message put back by Esc-Esc is an edit of the last turn: the turn it
    // replaces is dropped from the conversation, and the line's own rewind
    // undoes its file changes before this message runs.
    if model.take_replacing() {
        model.drop_last_turn();
        model.begin_turn(redact_param_values(&text));
        model.start_turn();
        model.remember_turn(text.clone());
        cmds.push(Cmd::ReplaceLastTurn(text));
        return;
    }
    send(model, text, cmds);
}

/// Sends one message: the turn opens here, because the operator's own words
/// are the one thing the event stream never carries.
pub(super) fn send(model: &mut Model, text: String, cmds: &mut Vec<Cmd>) {
    // The echo is the displayed copy of the line, redacted exactly as the ring
    // redacts it: what the run received and what the transcript shows are the
    // same line only when there is nothing to redact.
    model.begin_turn(redact_param_values(&text));
    model.start_turn();
    model.remember_turn(text.clone());
    cmds.push(Cmd::Send(text));
}
