use super::{ignore::IgnoreStack, io::ReadBudget, order::GlobHit, types::*};
use crate::workspace::{EntryKind, pattern::Pattern};
use regex::bytes::{Regex, RegexSet};
use std::time::SystemTime;

pub(super) struct Scan<'a> {
    pub(super) query: &'a SearchQuery,
    pub(super) limits: &'a SearchLimits,
    pub(super) control: Control<'a>,
    pub(super) scope: String,
    pub(super) glob: Pattern,
    pub(super) include: Option<Pattern>,
    pub(super) text: Option<RegexSet>,
    /// The whole-file matcher a `multiline` query states: the same pattern,
    /// matched against the file rather than each line, so a match can span a
    /// line boundary. Built instead of `text`, never beside it.
    pub(super) multiline_text: Option<Regex>,
    pub(super) folded: Option<String>,
    pub(super) ignores: IgnoreStack,
    pub(super) reads: ReadBudget,
    pub(super) kept_bytes: usize,
    /// Entries served that spend the query's entry cap: hits in content mode,
    /// files in the other two. A context line is served around a hit, so it
    /// is never counted here.
    pub(super) served: u64,
    /// Glob hits held for the mtime ordering, which decides what the match cap
    /// keeps. Bounded by `limits.entries`, the same walk budget that admits
    /// them, and by the 512-byte path bound the walk enforces.
    pub(super) pending: Vec<GlobHit>,
    pub(super) outcome: SearchOutcome,
    pub(super) cancelled: bool,
}

impl Scan<'_> {
    pub(super) fn alive(&mut self) -> bool {
        match self.control.check() {
            Ok(true) => true,
            Ok(false) => {
                self.outcome.incomplete(SearchStop::Deadline);
                false
            }
            Err(_) => {
                self.cancelled = true;
                false
            }
        }
    }
    pub(super) fn private(&self, path: &str) -> bool {
        crate::coding::snapshot::sensitive_path(path)
            || saya_store::screen_private_content(path, None).is_err()
            || saya_types::redact(path) != path
            || self.query.private_prefixes.iter().any(|prefix| {
                prefix.is_empty()
                    || prefix == "."
                    || path == prefix
                    || path
                        .strip_prefix(prefix)
                        .is_some_and(|tail| tail.starts_with('/'))
            })
    }
    pub(super) fn ignored(&mut self, path: &str, directory: bool) -> Option<bool> {
        match self.ignores.ignored(path, directory, &self.control) {
            Ok(Ok(ignored)) => Some(ignored),
            Ok(Err(stop)) => {
                self.outcome.incomplete(stop);
                None
            }
            Err(_) => {
                self.cancelled = true;
                None
            }
        }
    }
    pub(super) fn relative<'a>(&self, path: &'a str) -> &'a str {
        if self.scope.is_empty() {
            path
        } else if path == self.scope {
            path.rsplit('/').next().unwrap_or(path)
        } else {
            path.strip_prefix(&self.scope)
                .and_then(|tail| tail.strip_prefix('/'))
                .unwrap_or(path)
        }
    }
    /// Whether a `type` filter admits this entry kind. A query without one
    /// admits every kind the walk reaches.
    pub(super) fn admits(&self, kind: EntryKind) -> bool {
        self.query.entry.is_none_or(|entry| entry.admits(kind))
    }
    /// Whether the query's `type` extensions admit this path. A query without
    /// them admits every path. The decision is made before the file is read,
    /// so the read budget is spent only on files the call can serve — an
    /// unread file has no content to leak either way.
    pub(super) fn admits_extension(&self, path: &str) -> bool {
        let Some(extensions) = self.query.grep.extensions.as_deref() else {
            return true;
        };
        let name = path.rsplit('/').next().unwrap_or(path);
        // A leading dot names a hidden file, not an extension: `.gitignore`
        // has none, so no `type` admits it.
        name.rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .is_some_and(|(_, extension)| {
                extensions
                    .iter()
                    .any(|admitted| admitted.eq_ignore_ascii_case(extension))
            })
    }
    /// The entries this query serves before it stops keeping them: the stated
    /// `head_limit`, never above the match cap `SearchLimits` states.
    pub(super) fn cap(&self) -> usize {
        self.query
            .grep
            .head_limit
            .map_or(self.limits.matches, |limit| limit.min(self.limits.matches))
    }
    /// One entry the walk found by itself: it counts against `matches_found`
    /// and the entry cap, and is served unless the cap or the result byte
    /// budget refuses it. `clipped` says the entry's own text was cut to the
    /// line bound — the walk keeps going either way.
    pub(super) fn record_found(&mut self, hit: SearchMatch, clipped: bool) {
        self.outcome.matches_found += 1;
        if clipped {
            self.outcome.truncated = true;
            self.outcome
                .stop_reason
                .get_or_insert(SearchStop::ResultBytes);
        }
        if self.served as usize >= self.cap() {
            self.outcome.truncated = true;
            self.outcome.stop_reason.get_or_insert(SearchStop::Matches);
            return;
        }
        if self.serve(hit, clipped) {
            self.served += 1;
        }
    }
    /// One line served only as a hit's neighbour: it never spends the entry
    /// cap and is never counted as found. The result byte budget still bounds
    /// it.
    pub(super) fn record_context(&mut self, hit: SearchMatch, clipped: bool) {
        self.serve(hit, clipped);
    }
    fn serve(&mut self, hit: SearchMatch, clipped: bool) -> bool {
        if clipped {
            self.outcome.truncated = true;
            self.outcome
                .stop_reason
                .get_or_insert(SearchStop::ResultBytes);
        }
        let bytes = serde_json::to_vec(&hit)
            .map(|bytes| bytes.len() + 1)
            .unwrap_or(usize::MAX);
        if self.kept_bytes.saturating_add(bytes) > self.limits.result_bytes {
            self.outcome.truncated = true;
            self.outcome
                .stop_reason
                .get_or_insert(SearchStop::ResultBytes);
            return false;
        }
        self.kept_bytes += bytes;
        self.outcome.matches.push(hit);
        true
    }
    /// One glob path, held uncapped: the ordering needs every hit before the
    /// match cap can keep the newest ones.
    pub(super) fn record_path(&mut self, path: &str, modified: Option<SystemTime>) {
        self.outcome.matches_found += 1;
        self.pending.push(GlobHit {
            path: path.into(),
            modified,
        });
    }
}
