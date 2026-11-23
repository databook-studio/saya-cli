//! The panes that own the screen while they are open, and the model's side of
//! them.
//!
//! One pane at a time: it takes the keyboard before the card and the composer
//! ([`super::overlay_keys`]), and it is painted over the transcript
//! ([`super::run_draw_overlay`]). `/mode`'s picker is the first
//! ([`Overlay::Mode`]); `02` §11's command palette and key sheet
//! ([`Overlay::Palette`], [`Overlay::Help`]) sit beside it.
//!
//! The pane's own state lives with the pane (`view::mode_picker`); what this
//! module holds is the model's half — which pane is open, how the keyboard walks
//! it, and what a change says.

use saya_agent::PermissionMode;

use super::model::Model;
use super::view::ModePicker;
use super::view::Palette;
use super::view::mode_words;

/// A pane that owns the screen and the keyboard until it closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Overlay {
    /// `/mode` with no argument: the six modes on one screen (`02` §8).
    Mode(ModePicker),
    /// The command palette (`02` §11): `Ctrl+K`, or `/` typed into an empty
    /// composer. Its query and its selection are the pane's own.
    Palette(Palette),
    /// The key sheet (`02` §11): `?` typed into an empty composer, or `/help`.
    /// It draws the state the model is in, so it holds nothing of its own.
    Help,
    /// The background jobs drawer (`02` §8): `Ctrl+J` or `/job`. Its rows and
    /// its selection live on the model, because they are the run's jobs and
    /// stay current while the drawer is closed.
    Jobs,
    /// The startup workspace-trust question (`02` §9.4, T16): opened once when
    /// the session bound no workspace, answered `y`/`n`.
    Trust,
}

impl Model {
    /// The pane that owns the screen while it is open, if one does.
    pub(crate) fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    /// Whether a pane owns the screen and the keyboard.
    pub(crate) const fn overlay_open(&self) -> bool {
        self.overlay.is_some()
    }

    /// The mode picker, while it is the pane that is open.
    pub(crate) fn mode_picker(&self) -> Option<&ModePicker> {
        match self.overlay.as_ref()? {
            Overlay::Mode(picker) => Some(picker),
            Overlay::Palette(_) | Overlay::Help | Overlay::Jobs | Overlay::Trust => None,
        }
    }

    /// The mode picker, while it is the pane that is open, to be walked.
    pub(crate) fn mode_picker_mut(&mut self) -> Option<&mut ModePicker> {
        match self.overlay.as_mut()? {
            Overlay::Mode(picker) => Some(picker),
            Overlay::Palette(_) | Overlay::Help | Overlay::Jobs | Overlay::Trust => None,
        }
    }

    /// The command palette, while it is the pane that is open.
    pub(crate) fn palette(&self) -> Option<&Palette> {
        match self.overlay.as_ref()? {
            Overlay::Palette(palette) => Some(palette),
            Overlay::Mode(_) | Overlay::Help | Overlay::Jobs | Overlay::Trust => None,
        }
    }

    /// The command palette, while it is the pane that is open, to be walked.
    pub(crate) fn palette_mut(&mut self) -> Option<&mut Palette> {
        match self.overlay.as_mut()? {
            Overlay::Palette(palette) => Some(palette),
            Overlay::Mode(_) | Overlay::Help | Overlay::Jobs | Overlay::Trust => None,
        }
    }

    /// Opens `02` §11's command palette on an empty query: every command the
    /// catalogue lists is offered until the operator narrows it.
    pub(crate) fn open_palette(&mut self) {
        self.overlay = Some(Overlay::Palette(Palette::default()));
    }

    /// Opens `02` §11's key sheet: the whole keyboard, grouped, read at paint
    /// time rather than captured here. It opens at its first row.
    pub(crate) fn open_help(&mut self) {
        self.overlay = Some(Overlay::Help);
        self.help_scroll = 0;
    }

    /// How far the key sheet's body is scrolled.
    pub(crate) const fn help_scroll(&self) -> usize {
        self.help_scroll
    }

    /// Moves the key sheet's body by `rows`, a negative count moving it up. It
    /// never moves above its first row; the pane clamps the far end against the
    /// body it paints.
    pub(crate) fn scroll_help(&mut self, rows: isize) {
        let moved = self.help_scroll as isize + rows;
        self.help_scroll = moved.max(0) as usize;
    }

    /// Opens the workspace-trust question (`02` §9.4): the one decision a
    /// session that bound no workspace is asked, once, on its own screen.
    pub(crate) fn open_trust(&mut self) {
        self.overlay = Some(Overlay::Trust);
    }

    /// Whether the trust question is the pane that is open.
    pub(crate) fn trust_open(&self) -> bool {
        matches!(self.overlay, Some(Overlay::Trust))
    }

    /// Opens `/mode`'s picker on the mode in force (`02` §8).
    pub(crate) fn open_mode_picker(&mut self) {
        self.overlay = Some(Overlay::Mode(ModePicker::on(self.mode)));
    }

    /// Closes the open pane, whatever it was: the transcript is the screen
    /// again.
    pub(crate) fn close_overlay(&mut self) {
        self.overlay = None;
    }

    /// Says a mode change where it happened: the one-sentence toast of `02` §4,
    /// and — only once there is a conversation to say it in — one compact line
    /// in the transcript (`02` §3.10). On a screen with no turns the notice is
    /// not written, so the welcome screen a Shift+Tab arrived on survives it.
    pub(crate) fn note_mode(&mut self, mode: PermissionMode) {
        if !self.transcript.turns().is_empty() {
            let message = mode_words::line(mode);
            self.transcript
                .current()
                .push(crate::front::Entry::Notice(crate::front::Notice {
                    message,
                }));
        }
        self.say(mode_words::toast(mode));
    }
}
