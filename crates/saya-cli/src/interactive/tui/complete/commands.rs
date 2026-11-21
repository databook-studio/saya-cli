use super::Candidate;
use crate::slash::catalog::{self, Surface};

/// Candidates for a bare command word (no space yet): every catalogued name
/// the popup's surface carries whose name fuzzy-matches the prefix, with the
/// catalogue's one-line description. The catalogue is the only list — the
/// popup, `/help` and the parser read it — so a command added there appears
/// here without a second edit.
pub(super) fn command_candidates(
    prefix: &str,
    total_chars: usize,
) -> Option<(usize, usize, Vec<Candidate>)> {
    let mut scored: Vec<(i32, Candidate)> = catalog::entries()
        .filter(|entry| entry.surfaces.covers(Surface::Tui))
        .filter_map(|entry| {
            super::super::fuzzy::fuzzy_score(entry.name, prefix).map(|score| {
                (
                    score,
                    Candidate {
                        value: format!("/{}", entry.name),
                        description: Some(entry.description.to_string()),
                    },
                )
            })
        })
        .collect();

    if scored.is_empty() {
        return None;
    }
    scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let candidates = scored.into_iter().map(|(_, candidate)| candidate).collect();

    Some((0, total_chars, candidates))
}
