/// A single completion candidate for the slash popup.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) value: String,
    pub(crate) description: Option<String>,
}

#[allow(dead_code)]
const KNOWN_COMMANDS: &[(&str, &str)] = &[
    ("connect", "Connect to a database profile"),
    ("connections", "List configured database connections"),
    ("include", "Include a database profile in query scope"),
    ("exclude", "Exclude a database profile from query scope"),
    ("provider", "Set or view the AI provider"),
    ("model", "Set or view the AI model"),
    ("privacy", "Enable or disable data sharing privacy"),
    ("approvals", "Set approval policy for tool execution"),
    ("schema", "Inspect or refresh database schema"),
    ("sql", "Run a raw SQL query against the active profile"),
    ("clear", "Clear current session context"),
    ("history", "Show saved sessions"),
    ("sessions", "List saved sessions"),
    ("resume", "Resume a saved session by id"),
    ("help", "Show help for slash commands"),
    ("exit", "Exit the REPL"),
    ("quit", "Exit the REPL"),
];

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
        let mut scored: Vec<(i32, Candidate)> = KNOWN_COMMANDS
            .iter()
            .filter_map(|(name, desc)| {
                super::fuzzy::fuzzy_score(name, prefix).map(|score| {
                    (
                        score,
                        Candidate {
                            value: format!("/{name}"),
                            description: Some((*desc).to_string()),
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
    fn test_non_slash_line() {
        assert_eq!(slash_candidates("hello", &profiles()), None);
    }

    #[test]
    fn test_slash_only() {
        let (start, end, candidates) = slash_candidates("/", &profiles()).unwrap();
        assert_eq!((start, end), (0, 1));
        assert_eq!(candidates.len(), 17);
        assert_eq!(candidates[0].value, "/connect");
        assert_eq!(
            candidates[0].description.as_deref(),
            Some("Connect to a database profile")
        );
    }

    #[test]
    fn test_command_prefix() {
        let (start, end, candidates) = slash_candidates("/co", &profiles()).unwrap();
        assert_eq!((start, end), (0, 3));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["/connect", "/connections"]);
    }

    #[test]
    fn test_command_prefix_case_insensitive() {
        let (start, end, candidates) = slash_candidates("/CO", &profiles()).unwrap();
        assert_eq!((start, end), (0, 3));
        let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["/connect", "/connections"]);
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
