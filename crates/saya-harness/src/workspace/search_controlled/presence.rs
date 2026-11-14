//! A bounded presence check over the walk the searches use: does the
//! workspace hold any file carrying one extension?
//!
//! It answers a question about names and reads no file content, so it is the
//! cheap half of what a search pays for. The walk is the searches' own —
//! contained, symlink-refusing, and gitignore-aware through the same
//! [`IgnoreStack`] — and it stops at the first match. Both bounds are named,
//! so a caller that must decide something before it advertises a tool pays a
//! bounded, predictable price; a walk a bound stopped answers `complete:
//! false`, which a caller reads as "no answer", never as "no such file".

use super::{ignore::IgnoreStack, io::ReadBudget, types::Control};
use crate::{
    HarnessError,
    workspace::{
        EntryKind, Workspace,
        search_directory::Directory,
        walk::{WalkAction, WalkVisitor},
    },
};
use saya_agent::CancellationToken;
use std::time::{Duration, Instant, SystemTime};

/// The most entries one presence check visits before it stops without an
/// answer.
pub const PRESENCE_MAX_ENTRIES: usize = 4096;
/// How long one presence check may run before it stops without an answer.
pub const PRESENCE_TIMEOUT: Duration = Duration::from_millis(25);
/// The most ignore-policy bytes one presence check reads, across every
/// `.gitignore` it opens — the same ceiling `IgnoreStack` holds itself to.
pub const PRESENCE_MAX_IGNORE_BYTES: usize = 256 * 1024;

/// The bounds one presence check runs under. Named rather than fixed, so a
/// caller states what it is willing to pay for the answer and a test can
/// shrink either bound to prove what happens at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceBounds {
    pub entries: usize,
    pub timeout: Duration,
    pub ignore_bytes: usize,
}

impl Default for PresenceBounds {
    fn default() -> Self {
        Self {
            entries: PRESENCE_MAX_ENTRIES,
            timeout: PRESENCE_TIMEOUT,
            ignore_bytes: PRESENCE_MAX_IGNORE_BYTES,
        }
    }
}

/// What one presence check answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presence {
    /// Whether the walk reached a file carrying the extension.
    pub found: bool,
    /// How many entries the walk read before it stopped.
    pub entries_visited: usize,
    /// Whether a bound stopped the walk. `false` with `found` false is not a
    /// "no": it means the walk never reached an answer, and the workspace may
    /// still hold such a file.
    pub complete: bool,
}

impl Workspace {
    /// Whether the walk reaches a file whose extension is `extension` — the
    /// bare extension, without the dot, compared ASCII-case-insensitively.
    ///
    /// The walk is the searches' own: contained, symlink-refusing, and
    /// gitignore-aware, so a file the workspace's own ignore policy hides is
    /// not counted. The first match stops it. `found` false with `complete`
    /// false means a bound stopped the walk before it could answer — never
    /// that the workspace holds no such file.
    pub fn holds_file_extension(&self, extension: &str, bounds: PresenceBounds) -> Presence {
        let token = CancellationToken::new();
        let mut check = PresenceCheck {
            extension,
            bounds,
            control: Control {
                token: &token,
                deadline: Instant::now() + bounds.timeout,
            },
            ignores: IgnoreStack::new(0),
            reads: ReadBudget {
                used: 0,
                maximum: bounds.ignore_bytes,
            },
            entries_visited: 0,
            found: false,
            stopped: false,
        };
        if extension.is_empty() {
            check.stopped = true;
            return check.answer();
        }
        let directory = match Directory::open(self, "") {
            Ok(directory) => directory,
            Err(_) => {
                check.stopped = true;
                return check.answer();
            }
        };
        if self.walk_held("", directory, &mut check).is_err() {
            check.stopped = true;
        }
        check.answer()
    }
}

/// The walk's one visitor: it carries the ignore stack the walk needs, counts
/// every entry it reads, and stops the moment it meets a file carrying the
/// extension — or a bound.
struct PresenceCheck<'a> {
    extension: &'a str,
    bounds: PresenceBounds,
    control: Control<'a>,
    ignores: IgnoreStack,
    reads: ReadBudget,
    entries_visited: usize,
    found: bool,
    /// A bound, an ignore policy the walk could not read, or an unaddressable
    /// entry stopped the check before it could answer.
    stopped: bool,
}

impl PresenceCheck<'_> {
    fn answer(&self) -> Presence {
        Presence {
            found: self.found,
            entries_visited: self.entries_visited,
            complete: !self.stopped,
        }
    }

    /// Whether the check may go on. A deadline that has passed stops it, the
    /// same way the entry bound does.
    fn alive(&mut self) -> bool {
        let alive = matches!(self.control.check(), Ok(true));
        if !alive {
            self.stopped = true;
        }
        alive
    }
}

impl WalkVisitor for PresenceCheck<'_> {
    fn advance(&mut self) -> bool {
        if self.found || self.stopped {
            return false;
        }
        if self.entries_visited >= self.bounds.entries {
            self.stopped = true;
            return false;
        }
        self.alive()
    }

    fn seen(&mut self) -> Result<bool, HarnessError> {
        self.entries_visited += 1;
        Ok(true)
    }

    fn enter(&mut self, path: &str, directory: &Directory) -> Result<WalkAction, HarnessError> {
        if !self.alive() {
            return Ok(WalkAction::Stop);
        }
        // The searches' own ignore rule, over the same stack: a directory
        // `.gitignore` hides is not descended into, and a `.gitignore` the
        // walk cannot read stops the check rather than answering "not here".
        match self
            .ignores
            .push(path, directory, &mut self.reads, &self.control)
        {
            Ok(Ok(())) => Ok(WalkAction::Continue),
            Ok(Err(_)) | Err(_) => {
                self.stopped = true;
                Ok(WalkAction::Stop)
            }
        }
    }

    fn leave(&mut self) {
        self.ignores.pop();
    }

    fn entry(
        &mut self,
        path: &str,
        kind: EntryKind,
        _size: u64,
        _mtime: Option<SystemTime>,
        _directory: &Directory,
    ) -> Result<WalkAction, HarnessError> {
        if self.found || self.stopped {
            return Ok(WalkAction::Stop);
        }
        if !self.alive() {
            return Ok(WalkAction::Stop);
        }
        // The searches' own ignore rule, asked of the same stack `enter`
        // pushes: a name the workspace's policy hides is not a name this check
        // counted, and a policy the walk cannot read stops the check rather
        // than answering "not here".
        match self
            .ignores
            .ignored(path, kind == EntryKind::Dir, &self.control)
        {
            Ok(Ok(true)) => return Ok(WalkAction::Prune),
            Ok(Ok(false)) => {}
            Ok(Err(_)) | Err(_) => {
                self.stopped = true;
                return Ok(WalkAction::Stop);
            }
        }
        if kind == EntryKind::File && carries(self.extension, path) {
            self.found = true;
            return Ok(WalkAction::Stop);
        }
        Ok(WalkAction::Continue)
    }
}

/// Whether `path`'s own name carries `extension`. A leading dot names a
/// hidden file, not an extension, so `.ipynb` carries none — the same rule
/// the searches' `type` filter applies.
fn carries(extension: &str, path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .is_some_and(|(_, found)| found.eq_ignore_ascii_case(extension))
}
