use super::Candidate;

/// One-line description per command, shown in the popup. The popup reads it
/// from the single source in `slash::help` — see
/// [`crate::slash::COMMAND_DESCRIPTIONS`] — so the popup and the `/help` listing
/// share one copy and cannot drift. The thin local alias keeps the call site
/// short; a test asserts the shared table still covers exactly the parser's
/// registry.
pub(super) fn description_for(name: &str) -> Option<&'static str> {
    crate::slash::description_for(name)
}

/// Candidates for a bare command word (no space yet): every known command
/// whose name fuzzy-matches the prefix, with its shared description.
pub(super) fn command_candidates(
    prefix: &str,
    total_chars: usize,
) -> Option<(usize, usize, Vec<Candidate>)> {
    let mut scored: Vec<(i32, Candidate)> = crate::slash::registry::KNOWN_COMMANDS
        .iter()
        .filter_map(|name| {
            let description = description_for(name)?;
            super::super::fuzzy::fuzzy_score(name, prefix).map(|score| {
                (
                    score,
                    Candidate {
                        value: format!("/{name}"),
                        description: Some(description.to_string()),
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
