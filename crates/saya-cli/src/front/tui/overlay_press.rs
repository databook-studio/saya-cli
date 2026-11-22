//! What a pane's own keys do to the model.
//!
//! The panes that own the screen answer the presses `overlay_keys` reads, and
//! this module carries them out: opening `02` §11's palette and key sheet,
//! narrowing the palette's list, and closing a pane. The one that leaves the
//! surface — Enter on the palette's selected command — does not do anything of
//! its own: it puts the command in the composer and hands the line to the
//! composer's submit, so a chosen command and a typed one are one path.

use super::keys::Intent;
use super::model::Model;
use super::msg::Cmd;
use super::update_submit::submit;
use super::view::Palette;

/// Performs one of the panes' intents on `model`.
pub(crate) fn press(model: &mut Model, intent: Intent, cmds: &mut Vec<Cmd>) {
    match intent {
        Intent::OpenPalette => model.open_palette(),
        Intent::OpenHelp => model.open_help(),
        Intent::PaletteUp => walk(model, Palette::up),
        Intent::PaletteDown => walk(model, Palette::down),
        Intent::PaletteBackspace => walk(model, |pane| {
            pane.backspace();
        }),
        Intent::PaletteInsert(character) => walk(model, |pane| pane.push(character)),
        Intent::PaletteRun => run(model, cmds),
        Intent::PaletteClose | Intent::HelpClose => model.close_overlay(),
        // The panes' own intents are the only ones the update sends here.
        _ => {}
    }
}

/// One edit of the open palette, if it is the pane that is open.
fn walk<F>(model: &mut Model, edit: F)
where
    F: FnOnce(&mut Palette),
{
    if let Some(palette) = model.palette_mut() {
        edit(palette);
    }
}

/// Enter on the palette (`02` §11's `↵ insert`): the line the operator ends up
/// with is the one that runs — a chosen name and a typed one take one path, and
/// one dispatch.
///
/// A query that already names the command *and* carries the rest of the line —
/// `/start --id pty-job …`, `/approve pty-job`, which is how a job's consent is
/// given by name (`02` §8) — is that whole line, so it goes into the composer as
/// it was typed and runs. A query that names nothing yet inserts the selected
/// command for the operator to finish; one that asks for no argument at all runs
/// at once.
fn run(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let typed = model
        .palette()
        .map(|palette| palette.query().to_owned())
        .unwrap_or_default();
    let entry = model.palette().and_then(Palette::selected_entry);
    model.close_overlay();
    model.take_composer();
    let Some(entry) = entry else {
        // Nothing matched: the line is still the operator's, whole — and the
        // parser answers a line it cannot read, in its own words.
        model.insert(&format!("/{typed}"));
        submit(model, cmds);
        return;
    };
    if let Some(tail) = typed
        .strip_prefix(entry.name)
        .filter(|tail| !tail.trim().is_empty())
    {
        model.insert(&format!("/{}{tail}", entry.name));
        submit(model, cmds);
        return;
    }
    model.insert(&format!("/{} ", entry.name));
    if entry.takes_arguments() {
        return;
    }
    submit(model, cmds);
}
