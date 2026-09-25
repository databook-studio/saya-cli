use super::Candidate;

/// Candidates for a command argument (the line already has a space): profile
/// names for `/connect`/`/include`/`/exclude`, provider names, approval modes,
/// or privacy toggles — each fuzzy-matched against the partial argument.
pub(super) fn argument_candidates(
    cmd_word: &str,
    arg: &str,
    profiles: &[String],
    start_char: usize,
    total_chars: usize,
) -> Option<(usize, usize, Vec<Candidate>)> {
    let choices: Vec<&str> = match cmd_word {
        "connect" | "include" | "exclude" => profiles.iter().map(String::as_str).collect(),
        "provider" => vec![
            "ollama",
            "openai",
            "openai_compatible",
            "anthropic",
            "gemini",
        ],
        "approvals" => vec!["ask", "read-only", "never"],
        "privacy" => vec!["on", "off"],
        _ => return None,
    };

    let mut scored: Vec<(i32, Candidate)> = choices
        .into_iter()
        .filter_map(|val| {
            super::super::fuzzy::fuzzy_score(val, arg).map(|score| {
                (
                    score,
                    Candidate {
                        value: val.to_string(),
                        description: None,
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

    Some((start_char, total_chars, candidates))
}
