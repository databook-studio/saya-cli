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

/// Enter on the palette (`02` §11's `↵ insert`): the selected command takes the
/// composer's place, so the line Enter then runs is the composer's own — the
/// same line, and the same dispatch, an operator's typed command has.
///
/// A command the catalogue says needs an argument stops there: the composer
/// holds `/<name> ` for the operator to finish. One that asks for nothing
/// mandatory runs at once.
fn run(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let Some(entry) = model.palette().and_then(Palette::selected_entry) else {
        return;
    };
    model.close_overlay();
    model.take_composer();
    model.insert(&format!("/{} ", entry.name));
    if entry.takes_arguments() {
        return;
    }
    submit(model, cmds);
}
