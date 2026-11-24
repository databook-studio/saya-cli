//! Where a call runs, as the panel states it: the folder it runs in — or the
//! words that name it when it names no folder — and the safety fact that
//! travels with it.
//!
//! The line gives way at the front, never at the end. The last component of a
//! folder is the part an operator recognises, so it is the part the line keeps
//! whole: a path still too wide keeps its last components behind a leading
//! `…/`, and a last component wider than the line loses its middle rather than
//! its end. The home directory is the one prefix shortened without a mark,
//! because `~` says as much as the absolute path in fewer cells.
//!
//! A place can also carry a safety fact — a command's lane, a write's scope.
//! That fact is never dropped for width: it sits beside the place when the line
//! has room for it, and moves to a row of its own when it does not.

use std::borrow::Cow;

use super::cards::cells;

/// What a place separates its safety fact with.
const SEPARATOR: &str = " · ";

/// The ellipsis the rest of the surface marks a cut with.
const ELLIPSIS: char = '…';

/// The leading mark that says a path's front gave way.
const ELIDED: &str = "…/";

/// Where a call runs, as the panel states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    /// The words before the folder (`in `), or the whole statement when the
    /// place names no folder at all (`on connection demo`).
    head: String,
    /// The folder a call runs in, when the place names one.
    folder: Option<String>,
    /// The safety fact the place states beside it, never dropped from it.
    note: Option<&'static str>,
}

impl Place {
    /// A place that names the folder a call runs in.
    pub(crate) fn in_folder(folder: impl Into<String>, note: Option<&'static str>) -> Self {
        Self {
            head: String::from("in "),
            folder: Some(folder.into()),
            note,
        }
    }

    /// A place that names no folder: the words are the whole statement.
    pub(crate) fn words(words: impl Into<String>) -> Self {
        Self {
            head: words.into(),
            folder: None,
            note: None,
        }
    }

    /// The same place, stating a safety fact beside it.
    pub(crate) fn noting(mut self, note: &'static str) -> Self {
        self.note = Some(note);
        self
    }

    /// The place as one line, exactly as it states itself: what a caller with
    /// no width to fit to writes.
    pub(crate) fn line(&self) -> String {
        let mut text = self.stated();
        if let Some(note) = self.note {
            text.push_str(SEPARATOR);
            text.push_str(note);
        }
        text
    }

    /// The rows the panel paints at `room` cells: the folder shortened from the
    /// front so its last component is whole, and the safety fact beside it —
    /// when the line has room for it — or on a row of its own.
    pub(crate) fn rows(&self, room: usize) -> Vec<String> {
        self.rows_within(room, home_directory().as_deref())
    }

    /// The same rows, for a reader whose home directory is `home`: the home
    /// prefix reads as `~`, which is the one shortening with no mark to read.
    pub(crate) fn rows_within(&self, room: usize, home: Option<&str>) -> Vec<String> {
        let room = room.max(1);
        let place = match &self.folder {
            Some(folder) => {
                let room = room.saturating_sub(cells(&self.head));
                format!("{}{}", self.head, fit_path(folder, room, home))
            }
            None => self.head.clone(),
        };
        let Some(note) = self.note else {
            return vec![place];
        };
        let beside = format!("{place}{SEPARATOR}{note}");
        if cells(&beside) <= room {
            return vec![beside];
        }
        vec![place, note.to_owned()]
    }

    /// The place alone, without the safety fact it travels with.
    fn stated(&self) -> String {
        match &self.folder {
            Some(folder) => format!("{}{folder}", self.head),
            None => self.head.clone(),
        }
    }
}

/// The folder as one row of `room` cells: the home directory reads as `~`, and
/// a path still too wide gives way at the front — a leading [`ELIDED`] and as
/// many of the last components as the row holds, so the last one is whole.
fn fit_path(folder: &str, room: usize, home: Option<&str>) -> String {
    let shown = shortened_home(folder, home);
    if cells(&shown) <= room {
        return shown.into_owned();
    }
    let parts: Vec<&str> = shown.split('/').filter(|part| !part.is_empty()).collect();
    let Some((last, taken)) = parts.split_last() else {
        return middle(&shown, room);
    };
    if cells(last) > room {
        return middle(last, room);
    }
    let mut tail = (*last).to_owned();
    for part in taken.iter().rev() {
        let longer = format!("{part}/{tail}");
        if cells(&longer) + cells(ELIDED) > room {
            break;
        }
        tail = longer;
    }
    format!("{ELIDED}{tail}")
}

/// The folder with the reader's home directory written as `~`: the shortest
/// name that still says where it is.
fn shortened_home<'a>(folder: &'a str, home: Option<&str>) -> Cow<'a, str> {
    let home = home.map(|home| home.trim_end_matches('/'));
    let Some(home) = home.filter(|home| !home.is_empty()) else {
        return Cow::Borrowed(folder);
    };
    match folder.strip_prefix(home) {
        Some("") => Cow::Borrowed("~"),
        Some(rest) if rest.starts_with('/') => Cow::Owned(format!("~{rest}")),
        _ => Cow::Borrowed(folder),
    }
}

/// `text` as one row of `room` cells with its middle taken out: the end of a
/// name is what an operator recognises, so both ends stay and the middle gives
/// way — never the end.
fn middle(text: &str, room: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= room {
        return text.to_owned();
    }
    if room <= 1 {
        return ELLIPSIS.to_string();
    }
    let keep = room - 1;
    let front = keep / 2;
    let back = keep - front;
    let mut out: String = chars[..front].iter().collect();
    out.push(ELLIPSIS);
    out.extend(&chars[chars.len() - back..]);
    out
}

/// The home directory the environment states, as the folder is measured
/// against it: the one prefix the place line shortens without a mark.
fn home_directory() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| home.to_string_lossy().into_owned())
}
