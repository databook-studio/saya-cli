//! The `@` list, while it is open: the workspace paths the file tools list, the
//! row the arrows point at, and the one key that puts a path in the composer.
//!
//! The list itself is gathered off the loop's thread (`Cmd::Mentions`) and
//! arrives as `Msg::Mentions`, so a slow walk never holds a frame: what this
//! module holds is the answer, and what it answers is the filtering — on the
//! path, by the crate's own fuzzy match ([`crate::chat::mentions`]).

use super::model::Model;
use crate::chat::mentions;

/// The most rows the model keeps ready for the pane: the pane paints fewer on a
/// short screen, from this one ordering.
pub(crate) const MAX_ROWS: usize = 32;

/// The `@` completion's state while it is open.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Mention {
    /// Every path the run's workspace offered, as the walk returned them.
    pub(crate) paths: Vec<String>,
    /// Whether the list has been asked for and has not arrived yet: the pane
    /// says it is looking, rather than claiming the workspace holds nothing.
    pub(crate) waiting: bool,
    /// The row the arrows point at, inside the filtered rows.
    pub(crate) selected: usize,
}

impl Model {
    /// The `@` list, while it is open.
    pub(crate) fn mention(&self) -> Option<&Mention> {
        self.mention.as_ref()
    }

    /// The paths the open list offers for what is being typed, best first.
    pub(crate) fn mention_rows(&self) -> Vec<String> {
        let Some(list) = self.mention.as_ref() else {
            return Vec::new();
        };
        let query = mentions::query(self.composer()).unwrap_or_default();
        mentions::filtered(&list.paths, query, MAX_ROWS)
    }

    /// The path the open list would put in the composer now, if any.
    pub(crate) fn mention_chosen(&self) -> Option<String> {
        let rows = self.mention_rows();
        let list = self.mention.as_ref()?;
        rows.get(list.selected.min(rows.len().saturating_sub(1)))
            .cloned()
    }

    /// Opens the list for the mention being typed. Nothing to offer — no `@token`
    /// at the end of the composer — leaves it closed. Returns whether the paths
    /// must be gathered now, which is the one time the surface does any work.
    pub(crate) fn open_mention(&mut self) -> bool {
        if mentions::query(self.composer()).is_none() {
            return false;
        }
        match &self.mention {
            Some(_) => false,
            None => {
                self.mention = Some(Mention {
                    waiting: true,
                    ..Mention::default()
                });
                true
            }
        }
    }

    /// Closes the list: a space, an Enter, an Esc, or a path accepted.
    pub(crate) fn close_mention(&mut self) {
        self.mention = None;
    }

    /// The list the walk answered with. A list the operator has already closed
    /// is dropped: the answer is only ever for the list that asked.
    pub(crate) fn fill_mentions(&mut self, paths: Vec<String>) {
        if let Some(list) = &mut self.mention {
            list.paths = paths;
            list.waiting = false;
            list.selected = 0;
        }
    }

    /// Moves the open list's selection, a negative count moving up, never past
    /// either end.
    pub(crate) fn move_mention(&mut self, steps: isize) {
        let rows = self.mention_rows().len();
        let Some(list) = &mut self.mention else {
            return;
        };
        if rows == 0 {
            list.selected = 0;
            return;
        }
        let moved = list.selected as isize + steps;
        list.selected = moved.clamp(0, rows as isize - 1) as usize;
    }

    /// Puts the selected path in the composer, replacing the token being typed,
    /// and closes the list. False when there is nothing to accept.
    pub(crate) fn accept_mention(&mut self) -> bool {
        let Some(path) = self.mention_chosen() else {
            return false;
        };
        if mentions::query(self.composer()).is_none() {
            return false;
        }
        self.composer = mentions::accepted(self.composer(), &path);
        self.close_mention();
        true
    }
}
