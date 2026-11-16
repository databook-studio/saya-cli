use super::{io::FileRead, state::Scan, types::*};
use crate::{
    HarnessError,
    workspace::{
        EntryKind,
        search_directory::Directory,
        walk::{WalkAction, WalkVisitor},
    },
};
use std::time::SystemTime;

impl WalkVisitor for Scan<'_> {
    fn advance(&mut self) -> bool {
        if !self.alive() {
            return false;
        }
        if self.outcome.entries_visited >= self.limits.entries {
            self.outcome.incomplete(SearchStop::Entries);
            return false;
        }
        true
    }
    fn seen(&mut self) -> Result<bool, HarnessError> {
        self.outcome.entries_visited += 1;
        Ok(true)
    }
    fn unavailable(&mut self) {
        self.outcome.files_skipped += 1;
        self.outcome.incomplete(SearchStop::Unavailable);
    }
    fn enter(&mut self, path: &str, directory: &Directory) -> Result<WalkAction, HarnessError> {
        if !self.alive() {
            return Ok(WalkAction::Stop);
        }
        let depth = if path.is_empty() {
            0
        } else {
            path.split('/').count()
        };
        if depth >= self.limits.depth {
            self.outcome.incomplete(SearchStop::Depth);
            return Ok(WalkAction::Prune);
        }
        match self
            .ignores
            .push(path, directory, &mut self.reads, &self.control)
        {
            Ok(Ok(())) => Ok(WalkAction::Continue),
            Ok(Err(stop)) => {
                self.outcome.incomplete(stop);
                Ok(WalkAction::Prune)
            }
            Err(_) => {
                self.cancelled = true;
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
        mtime: Option<SystemTime>,
        directory: &Directory,
    ) -> Result<WalkAction, HarnessError> {
        if !self.alive() {
            return Ok(WalkAction::Stop);
        }
        if path.len() > 512 {
            self.outcome.incomplete(SearchStop::Unavailable);
            return Ok(WalkAction::Prune);
        }
        if self.private(path) {
            return Ok(WalkAction::Prune);
        }
        match self.ignored(path, kind == EntryKind::Dir) {
            Some(true) => return Ok(WalkAction::Prune),
            Some(false) => {}
            None => return Ok(WalkAction::Stop),
        }
        if !self.alive() {
            return Ok(WalkAction::Stop);
        }
        if kind == EntryKind::Dir {
            if matches!(self.query.kind, SearchKind::Glob)
                && self.admits(kind)
                && self.glob.matches(self.relative(path))
            {
                self.record_path(path, mtime);
            }
            if path.split('/').count() >= self.limits.depth {
                self.outcome.incomplete(SearchStop::Depth);
                return Ok(WalkAction::Prune);
            }
            return Ok(WalkAction::Continue);
        }
        if kind != EntryKind::File {
            self.unavailable();
            return Ok(WalkAction::Prune);
        }
        // A glob that asked only for directories never reads file content: the
        // filters decide before the read, so the read budget is spent on
        // entries the call can serve. Nothing unfiltered can leak either way —
        // an unread file has no content to screen.
        if !self.admits(kind) || !self.admits_extension(path) {
            return Ok(WalkAction::Continue);
        }
        if self
            .include
            .as_ref()
            .is_some_and(|include| !include.matches(self.relative(path)))
        {
            return Ok(WalkAction::Continue);
        }
        let bytes = match self
            .reads
            .read(directory, path, self.limits.file_bytes, &self.control)
        {
            Ok(Ok(FileRead::Bytes(bytes))) => bytes,
            Ok(Ok(_)) => {
                self.unavailable();
                return Ok(WalkAction::Continue);
            }
            Ok(Err(stop)) => {
                self.outcome.incomplete(stop);
                return Ok(WalkAction::Stop);
            }
            Err(_) => {
                self.cancelled = true;
                return Ok(WalkAction::Stop);
            }
        };
        if saya_store::screen_private_content(path, Some(&bytes)).is_err() {
            return Ok(WalkAction::Continue);
        }
        if matches!(self.query.kind, SearchKind::Glob) {
            self.outcome.files_scanned += 1;
            if self.glob.matches(self.relative(path)) {
                self.record_path(path, mtime);
            }
        } else if let Ok(text) = std::str::from_utf8(&bytes) {
            self.outcome.files_scanned += 1;
            self.grep_text(path, text);
            if self.cancelled {
                return Ok(WalkAction::Stop);
            }
        } else {
            self.unavailable();
        }
        Ok(WalkAction::Continue)
    }
}
