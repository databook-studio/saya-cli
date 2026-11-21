//! The panes that own the screen while they are open, and the model's side of
//! them.
//!
//! One pane at a time: it takes the keyboard before the card and the composer
//! ([`super::keys`]), and it is painted over the transcript
//! ([`super::run_draw_overlay`]). `/mode`'s picker is the first
//! ([`Overlay::Mode`]); the palette, help, sessions and jobs panes T10-T17 built
//! join it beside it.
//!
//! The pane's own state lives with the pane (`view::mode_picker`); what this
//! module holds is the model's half — which pane is open, how the keyboard walks
//! it, and what a change says.

use saya_agent::PermissionMode;

use super::model::Model;
use super::view::ModePicker;
use super::view::mode_words;

/// A pane that owns the screen and the keyboard until it closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Overlay {
    /// `/mode` with no argument: the six modes on one screen (`02` §8).
    Mode(ModePicker),
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
            Overlay::Jobs | Overlay::Trust => None,
        }
    }

    /// The mode picker, while it is the pane that is open, to be walked.
    pub(crate) fn mode_picker_mut(&mut self) -> Option<&mut ModePicker> {
        match self.overlay.as_mut()? {
            Overlay::Mode(picker) => Some(picker),
            Overlay::Jobs | Overlay::Trust => None,
        }
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

    /// Says a mode change where it happened: one compact line in the
    /// transcript (`02` §3.10) and the one-sentence toast of `02` §4. Both come
    /// from the picker's own word table, so the transcript, the toast and the
    /// pane cannot disagree about a mode's words (J6).
    pub(crate) fn note_mode(&mut self, mode: PermissionMode) {
        let message = mode_words::line(mode);
        self.transcript
            .current()
            .push(crate::front::Entry::Notice(crate::front::Notice {
                message,
            }));
        self.say(mode_words::toast(mode));
    }
}
