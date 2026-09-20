use super::*;

fn profiles() -> Vec<String> {
    vec!["dev".to_string(), "prod".to_string()]
}

#[test]
fn descriptions_cover_exactly_the_registry() {
    // the popup reads its descriptions from the single source in
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
    assert_eq!(candidates.len(), 40); // registry size: every known command with a description
    assert_eq!(candidates[0].value, "/connect");
    // The description is the single-source one from slash::help, sharpened
    // to carry the /connect vs /include contrast (one replaces the
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
    // "co" prefixes connect, connections, compact, columns, contracts,
    // contract, confirm; all tie on score, so the stable sort keeps
    // registry order.
    assert_eq!(
        values,
        vec![
            "/connect",
            "/connections",
            "/compact",
            "/columns",
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
            "/compact",
            "/columns",
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
