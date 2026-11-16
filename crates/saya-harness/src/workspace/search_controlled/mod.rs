//! Scoped search owns bounded work, coverage truth and canonical private
//! policy; the presence check beside it reuses the same walk and ignore
//! policy to answer whether the workspace holds a file of one extension.
mod types;
pub use types::{
    GrepOptions, MAX_CONTEXT, MIN_RESULT_BYTES, SearchEntry, SearchError, SearchKind, SearchLimits,
    SearchMatch, SearchOutcome, SearchOutput, SearchQuery, SearchStop,
};
mod grep_scan;
mod ignore;
mod ignore_translation;
mod io;
mod order;
mod presence;
pub use presence::{
    PRESENCE_MAX_ENTRIES, PRESENCE_MAX_IGNORE_BYTES, PRESENCE_TIMEOUT, Presence, PresenceBounds,
};
mod scan;
mod state;
mod visit;
use crate::workspace::Workspace;
use saya_agent::CancellationToken;
use types::{MAX_EXTENSION_BYTES, MAX_EXTENSIONS};

impl Workspace {
    pub fn search_controlled(
        &self,
        query: &SearchQuery,
        limits: &SearchLimits,
        cancellation: &CancellationToken,
    ) -> Result<SearchOutcome, SearchError> {
        limits.validate()?;
        validate(self.root(), query, limits)?;
        if cancellation.is_cancelled() {
            return Err(SearchError::Cancelled);
        }
        scan::execute(self, query, limits, cancellation)
    }
}
fn validate(
    root: &std::path::Path,
    query: &SearchQuery,
    limits: &SearchLimits,
) -> Result<(), SearchError> {
    if query.pattern.is_empty()
        || query.pattern.len() > 1024
        || query.pattern.contains('\0')
        || query
            .include
            .as_ref()
            .is_some_and(|include| include.len() > 1024 || include.contains('\0'))
        || query.private_prefixes.len() > 4096
        || query
            .private_prefixes
            .iter()
            .map(String::len)
            .sum::<usize>()
            > 256 * 1024
    {
        return Err(SearchError::Invalid);
    }
    if matches!(query.kind, SearchKind::Glob)
        && (query.pattern.starts_with('/')
            || query
                .pattern
                .split('/')
                .any(|part| part == ".." || part.eq_ignore_ascii_case(".git")))
    {
        return Err(SearchError::Invalid);
    }
    valid_path(root, &query.path)?;
    for prefix in &query.private_prefixes {
        valid_path(root, prefix)?;
    }
    if matches!(query.kind, SearchKind::Glob) && (query.regex || query.include.is_some()) {
        return Err(SearchError::Invalid);
    }
    if matches!(query.kind, SearchKind::Grep) && query.entry.is_some() {
        return Err(SearchError::Invalid);
    }
    if matches!(query.kind, SearchKind::Glob) && query.grep != GrepOptions::default() {
        return Err(SearchError::Invalid);
    }
    // The entry cap never rises above the match cap `SearchLimits` states, and
    // the context bound is the fixed one. `context` with `multiline` is
    // undefined rather than ignored: the two would disagree about which line
    // a hit is anchored to.
    if query.grep.context > MAX_CONTEXT
        || (query.grep.multiline && query.grep.context > 0)
        || query
            .grep
            .head_limit
            .is_some_and(|limit| limit == 0 || limit > limits.matches)
        || query.grep.extensions.as_ref().is_some_and(|extensions| {
            extensions.is_empty()
                || extensions.len() > MAX_EXTENSIONS
                || extensions.iter().any(|extension| {
                    extension.is_empty()
                        || extension.len() > MAX_EXTENSION_BYTES
                        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
        })
    {
        return Err(SearchError::Invalid);
    }
    Ok(())
}
fn valid_path(root: &std::path::Path, path: &str) -> Result<(), SearchError> {
    if path.is_empty() || path == "." {
        return Ok(());
    }
    if path.len() > 512
        || path.split('/').any(|part| part == "..")
        || super::contain::validate_argument(root, path).is_err()
    {
        return Err(SearchError::Invalid);
    }
    Ok(())
}
