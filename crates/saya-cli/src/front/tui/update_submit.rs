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
    // A line the surface answers itself: bare `/mode` opens the picker and bare
    // `/job` opens the jobs drawer, neither of them sending anything
    // (`02` §8). Both are read before the slash dispatch, because the crate's
    // parser answers `/job` with its verbs — and bare `/job` has none.
    if line == SLASH_MODE {
        model.open_mode_picker();
        return;
    }
    if line == SLASH_JOB {
        model.open_jobs();
        return;
    }
    if line == SLASH_HELP {
        model.open_help();
        return;
    }
    // A line the surface answers itself — `/sql`, `/export`, `/chart`,
    // `/investigation` — is one command, never a turn (`03` §4.1). Every other
    // line is a turn's.
    match direct::read(model, &text) {
        direct::Read::Cmd(cmd) => {
            // The line opens the turn its answer lands in: a command's own
            // words are echoed where its rows will be, never only in the
            // composer that has just been cleared.
            model.begin_turn(shown);
            cmds.push(cmd);
            return;
        }
        direct::Read::Said(words) => {
            // A line the surface answers with words instead of running — a
            // refusal, a pointer — is echoed the same way, redacted.
            model.begin_turn(shown);
            model.say(words);
            return;
        }
        direct::Read::Turn => {}
    }
    // A command the surface does not answer itself is the session's own
    // operation, read by the chat layer's one table (A9) — never a turn. A
    // `Coding` command and `/mode` are the exceptions: both are the session's
    // own lines, read (and dispatched) by the run the surface is composed over
    // (`/job show <id>` is T15's proved case), so they stay turns here.
    if let Ok(Some(command)) = parse_slash_command(line)
        && !matches!(command, SlashCommand::Coding(_) | SlashCommand::Mode(_))
    {
        cmds.push(Cmd::Op(command));
        return;
    }
    if model.is_busy() {
        if model.queue(text).is_some() {
            model.say(QUEUED_REPLACED);
        }
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
    cmds.push(Cmd::Send(text));
}
