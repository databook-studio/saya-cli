use super::{
    ignore::{IgnoreStack, bounded_regex, bounded_set},
    io::ReadBudget,
    order,
    state::Scan,
    types::*,
};
use crate::workspace::{
    EntryKind, Workspace, pattern::Pattern, search_directory::Directory, walk::WalkVisitor,
};
use std::time::Instant;

enum Scope {
    Directory(Directory),
    File(Directory),
}

pub(super) fn execute(
    workspace: &Workspace,
    query: &SearchQuery,
    limits: &SearchLimits,
    cancellation: &saya_agent::CancellationToken,
) -> Result<SearchOutcome, SearchError> {
    let control = Control {
        token: cancellation,
        deadline: Instant::now() + limits.timeout,
    };
    if !control.check()? {
        let mut outcome = SearchOutcome::default();
        outcome.incomplete(SearchStop::Deadline);
        return Ok(outcome);
    }
    // A multiline query matches the whole file, so it compiles the one
    // matcher that reports where a match is; every other query matches line
    // by line and needs only the set.
    let text = if query.regex && !query.grep.multiline {
        let compiled = bounded_set(std::slice::from_ref(&query.pattern), query.case_insensitive);
        if !control.check()? {
            let mut outcome = SearchOutcome::default();
            outcome.incomplete(SearchStop::Deadline);
            return Ok(outcome);
        }
        Some(compiled.map_err(|_| SearchError::Invalid)?)
    } else {
        None
    };
    let multiline_text = if query.grep.multiline {
        // A literal pattern is escaped, so it stays a literal — one that may
        // hold the newline it matches across.
        let pattern = if query.regex {
            query.pattern.clone()
        } else {
            regex::escape(&query.pattern)
        };
        if !control.check()? {
            let mut outcome = SearchOutcome::default();
            outcome.incomplete(SearchStop::Deadline);
            return Ok(outcome);
        }
        Some(bounded_regex(&pattern, query.case_insensitive).map_err(|_| SearchError::Invalid)?)
    } else {
        None
    };
    let mut scan = Scan {
        query,
        limits,
        control,
        scope: if query.path == "." {
            String::new()
        } else {
            query.path.clone()
        },
        glob: Pattern::new(&query.pattern),
        include: query.include.as_deref().map(Pattern::new),
        folded: (query.case_insensitive && !query.grep.multiline)
            .then(|| query.pattern.to_lowercase()),
        ignores: IgnoreStack::new(usize::from(text.is_some() || multiline_text.is_some())),
        text,
        multiline_text,
        reads: ReadBudget {
            used: 0,
            maximum: limits.read_bytes,
        },
        kept_bytes: 512,
        served: 0,
        pending: Vec::new(),
        outcome: SearchOutcome::default(),
        cancelled: false,
    };
    if scan.alive() && !scan.private(&scan.scope) {
        match scan.prepare_scope(workspace)? {
            Some(Scope::Directory(directory)) => {
                if workspace
                    .walk_held(&scan.scope.clone(), directory, &mut scan)
                    .is_err()
                {
                    scan.outcome.incomplete(SearchStop::Unavailable);
                }
            }
            Some(Scope::File(directory)) if scan.advance() => {
                let scope = scan.scope.clone();
                // The scope named one file, so there is no directory entry to
                // carry a size or a time: the scope walk already established
                // the kind, and this branch is grep-only — a glob never takes
                // it, and grep reads no time.
                if scan
                    .seen()
                    .and_then(|_| scan.entry(&scope, EntryKind::File, 0, None, &directory))
                    .is_err()
                {
                    scan.outcome.incomplete(SearchStop::Unavailable);
                }
            }
            _ => {}
        }
    }
    if cancellation.is_cancelled() || scan.cancelled {
        return Err(SearchError::Cancelled);
    }
    if !scan.control.check()? {
        scan.outcome.incomplete(SearchStop::Deadline);
    }
    scan.outcome.total_matches = scan
        .outcome
        .coverage_complete
        .then_some(scan.outcome.matches_found);
    if matches!(query.kind, SearchKind::Glob) {
        // The mtime order is settled before the match cap, so the cap keeps
        // the newest hits; the byte cap below then drops from the tail, which
        // is the oldest. Grep keeps the path/line order its own cap assumes.
        let (matches, capped) = order::serve(std::mem::take(&mut scan.pending), limits.matches);
        scan.outcome.matches = matches;
        if capped {
            scan.outcome.truncated = true;
            scan.outcome.stop_reason.get_or_insert(SearchStop::Matches);
        }
    } else {
        scan.outcome
            .matches
            .sort_by(|left, right| left.key().cmp(&right.key()));
    }
    // The cap bounds the whole outcome. When the coverage detail alone cannot
    // fit it, that detail is dropped and the outcome marked — the call still
    // returns real matches rather than refusing. The matches are held
    // aside for the measurement so the decision is about the metadata, not
    // about how many hits the tail happens to hold.
    let held = std::mem::take(&mut scan.outcome.matches);
    let metadata_fits = serde_json::to_vec(&scan.outcome)
        .map_err(|_| SearchError::Invalid)?
        .len()
        <= limits.result_bytes;
    scan.outcome.matches = held;
    if !metadata_fits {
        scan.outcome.drop_coverage();
    }
    while serde_json::to_vec(&scan.outcome)
        .map_err(|_| SearchError::Invalid)?
        .len()
        > limits.result_bytes
    {
        if scan.outcome.matches.pop().is_none() {
            // `SearchLimits::validate` admits only caps that hold a
            // coverage-dropped outcome, so this is unreachable; a silent
            // oversized answer would be worse than refusing.
            break;
        }
        scan.outcome.truncated = true;
        scan.outcome
            .stop_reason
            .get_or_insert(SearchStop::ResultBytes);
    }
    // Every entry the walk found but the outcome does not serve, whichever
    // bound or cap left it out. A context line is served around a hit rather
    // than found, so it is never counted here. Exact when the coverage
    // completed, a lower bound otherwise — the same distinction
    // `coverage_complete` already draws.
    scan.outcome.omitted = scan.outcome.matches_found.saturating_sub(
        scan.outcome
            .matches
            .iter()
            .filter(|hit| hit.found())
            .count() as u64,
    );
    Ok(scan.outcome)
}

impl Scan<'_> {
    fn prepare_scope(&mut self, workspace: &Workspace) -> Result<Option<Scope>, SearchError> {
        let parts: Vec<_> = self
            .scope
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect();
        // Bound ancestor work itself, before opening or compiling parent policy.
        if parts.len() > self.limits.depth {
            self.outcome.incomplete(SearchStop::Depth);
            return Ok(None);
        }
        let mut directory = match Directory::open(workspace, "") {
            Ok(directory) => directory,
            Err(_) => {
                self.outcome.incomplete(SearchStop::Unavailable);
                return Ok(None);
            }
        };
        if parts.is_empty() {
            return Ok(Some(Scope::Directory(directory)));
        }
        let mut base = String::new();
        for (index, name) in parts.iter().enumerate() {
            if !self.alive() {
                return Ok(None);
            }
            if let Err(stop) =
                self.ignores
                    .push(&base, &directory, &mut self.reads, &self.control)?
            {
                self.outcome.incomplete(stop);
                return Ok(None);
            }
            let path = if base.is_empty() {
                name.clone()
            } else {
                format!("{base}/{name}")
            };
            if self.private(&path) {
                return Ok(None);
            }
            let kind = match directory.kind(workspace, &path, name) {
                Ok(kind) => kind,
                Err(_) => {
                    self.outcome.incomplete(SearchStop::Unavailable);
                    return Ok(None);
                }
            };
            match self.ignored(&path, kind == EntryKind::Dir) {
                Some(false) => {}
                _ => return Ok(None),
            }
            if index + 1 == parts.len()
                && kind == EntryKind::File
                && matches!(self.query.kind, SearchKind::Grep)
            {
                return Ok(Some(Scope::File(directory)));
            }
            if kind != EntryKind::Dir {
                return Err(SearchError::Invalid);
            }
            directory = match directory.child(workspace, &path, name) {
                Ok(child) => child,
                Err(_) => {
                    self.outcome.incomplete(SearchStop::Unavailable);
                    return Ok(None);
                }
            };
            base = path;
        }
        Ok(Some(Scope::Directory(directory)))
    }
}
