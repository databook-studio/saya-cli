/// A single completion candidate for the slash popup.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) value: String,
    pub(crate) description: Option<String>,
}

/// One-line description per command, shown in the popup. The popup reads it
/// from the single source in `slash::help` — see
/// [`crate::slash::COMMAND_DESCRIPTIONS`] — so the popup and the `/help` listing
/// share one copy and cannot drift. The thin local alias keeps the call site
/// short; a test asserts the shared table still covers exactly the parser's
/// registry.
fn description_for(name: &str) -> Option<&'static str> {
    crate::slash::description_for(name)
}

/// Given the current input line, returns the candidates for the slash popup plus
/// the half-open CHAR range [start, end) in `line` that accepting a candidate
/// replaces. Returns None when the popup should not be shown (line does not
/// start with '/', or the command takes no completable argument).
#[allow(dead_code)]
pub(crate) fn slash_candidates(
    line: &str,
    profiles: &[String],
) -> Option<(usize, usize, Vec<Candidate>)> {
    if !line.starts_with('/') {
        return None;
    }

    let total_chars = line.chars().count();

    if let Some(byte_idx) = line.rfind(' ') {
        let cmd_word = line[1..]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_lowercase();

        let choices: Vec<&str> = match cmd_word.as_str() {
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

        let arg = &line[byte_idx + 1..];
        let mut scored: Vec<(i32, Candidate)> = choices
            .into_iter()
            .filter_map(|val| {
                super::fuzzy::fuzzy_score(val, arg).map(|score| {
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

        let start_char = line[..byte_idx].chars().count() + 1;
        Some((start_char, total_chars, candidates))
    } else {
        let prefix = &line[1..];
        let mut scored: Vec<(i32, Candidate)> = crate::slash::registry::KNOWN_COMMANDS
            .iter()
            .filter_map(|name| {
                let description = description_for(name)?;
                super::fuzzy::fuzzy_score(name, prefix).map(|score| {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles() -> Vec<String> {
        vec!["dev".to_string(), "prod".to_string()]
    }

    #[test]
    fn descriptions_cover_exactly_the_registry() {
        // S17: the popup reads its descriptions from the single source in
        // `slash::help` (no local copy here), so this asserts that shared
        // table covers exactly the parser's registry — the popup and the
        // `/help` listing cannot drift, because they share this one table.
        let descriptions = crate::slash::COMMAND_DESCRIPTIONS;
        assert_eq!(
            descriptions.len(),
            crate::slash::registry::KNOWN_COMMANDS.len()
        );
        for (name, _) in descriptions {
            assert!(
                crate::slash::registry::KNOWN_COMMANDS.contains(name),
                "{name} described but not registered"
            );
        }
        for name in crate::slash::registry::KNOWN_COMMANDS {
            assert!(
                description_for(name).is_some(),
                "{name} registered but not described"
            );
        }
    }

    #[test]
    fn test_non_slash_line() {
        assert_eq!(slash_candidates("hello", &profiles()), None);
    }

    #[test]
    fn test_slash_only() {
        let (start, end, candidates) = slash_candidates("/", &profiles()).unwrap();
        assert_eq!((start, end), (0, 1));
        assert_eq!(candidates.len(), 28);
        assert_eq!(candidates[0].value, "/connect");
        // The description is the single-source one from slash::help, sharpened
        // in S17 to carry the /connect vs /include contrast (one replaces the
        // active profile, one adds a secondary).
        assert_eq!(
            candidates[0].description.as_deref(),
            Some("Replace the active database profile")
        );
    }

    #[test]
    fn test_command_prefix() {
        let (start, end, candidates) = slash_candidates("/co", &profiles()).unwrap();
        assert_eq!((start, end), (0, 3));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        // "co" prefixes connect, connections, contracts, contract, confirm;
        // all tie on score, so the stable sort keeps registry order.
        assert_eq!(
            values,
            vec![
                "/connect",
                "/connections",
                "/contracts",
                "/contract",
                "/confirm",
                "/doctor"
            ]
        );
    }

    #[test]
    fn test_command_prefix_case_insensitive() {
        let (start, end, candidates) = slash_candidates("/CO", &profiles()).unwrap();
        assert_eq!((start, end), (0, 3));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "/connect",
                "/connections",
                "/contracts",
                "/contract",
                "/confirm",
                "/doctor"
            ]
        );
    }

    #[test]
    fn test_profile_arguments() {
        let (start, end, candidates) = slash_candidates("/connect ", &profiles()).unwrap();
        assert_eq!((start, end), (9, 9));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["dev", "prod"]);
        assert_eq!(candidates[0].description, None);

        let (start, end, candidates_p) = slash_candidates("/connect p", &profiles()).unwrap();
        assert_eq!((start, end), (9, 10));
        let values_p: Vec<_> = candidates_p.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values_p, vec!["prod"]);
    }

    #[test]
    fn test_provider_arguments() {
        let (start, end, candidates) = slash_candidates("/provider op", &profiles()).unwrap();
        assert_eq!((start, end), (10, 12));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        // Fuzzy ranks the prefix matches first; weaker subsequence matches
        // (e.g. "anthropic" via o…p) may follow.
        assert_eq!(&values[..2], &["openai", "openai_compatible"]);
    }

    #[test]
    fn test_approvals_arguments() {
        let (start, end, candidates) = slash_candidates("/approvals ", &profiles()).unwrap();
        assert_eq!((start, end), (11, 11));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["ask", "read-only", "never"]);
    }

    #[test]
    fn test_privacy_arguments() {
        let (start, end, candidates) = slash_candidates("/privacy o", &profiles()).unwrap();
        assert_eq!((start, end), (9, 10));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["on", "off"]);
    }

    #[test]
    fn test_other_command_arguments() {
        assert_eq!(slash_candidates("/clear x", &profiles()), None);
    }

    #[test]
    fn test_multibyte_prefix() {
        let unicode_profiles = vec!["🦀dev".to_string(), "prod".to_string()];
        let (start, end, candidates) = slash_candidates("/connect 🦀", &unicode_profiles).unwrap();
        assert_eq!((start, end), (9, 10));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["🦀dev"]);

        assert_eq!(slash_candidates("/connect 🚀", &unicode_profiles), None);
    }
}
