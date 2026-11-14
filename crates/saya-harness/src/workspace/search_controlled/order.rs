//! Glob ordering: newest modified first, so the first hit is the file changed
//! last; a match cap keeps the newest hits rather than an arbitrary portion of
//! the walk.
use super::types::SearchMatch;
use std::time::SystemTime;

/// One glob hit: its path, and the modification time the walk's own stat
/// reported for it. `None` means that stat named no usable time — a filesystem
/// that refuses one, or a value the clock cannot hold. The time never reaches
/// the wire; it only orders the paths.
pub(super) struct GlobHit {
    pub(super) path: String,
    pub(super) modified: Option<SystemTime>,
}

/// Orders `hits` newest first — an unusable time sorts last, and every tie
/// breaks by path, so the order is total and deterministic — then applies the
/// match cap, so what the cap drops is the oldest. Reports whether the cap bit.
pub(super) fn serve(mut hits: Vec<GlobHit>, cap: usize) -> (Vec<SearchMatch>, bool) {
    hits.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.path.cmp(&right.path))
    });
    let truncated = hits.len() > cap;
    hits.truncate(cap);
    (
        hits.into_iter()
            .map(|hit| SearchMatch::Path(hit.path))
            .collect(),
        truncated,
    )
}

#[cfg(test)]
#[path = "order_tests.rs"]
mod tests;
