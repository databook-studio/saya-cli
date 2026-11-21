use super::*;

fn profiles() -> Vec<String> {
    vec!["dev".to_string(), "prod".to_string()]
}

#[test]
fn descriptions_cover_exactly_the_catalogue() {
    // the popup reads its rows from the catalogue — one table, no local
    // copy — so this asserts the catalogue's names and descriptions are the
    // popup's: every name is unique, every name carries a description, and
    // the count is today's 61 (53 commands and their 8 alias spellings).
    let entries: Vec<_> = crate::slash::catalog::entries().collect();
    assert_eq!(entries.len(), 61);
    for entry in &entries {
        assert!(
            entry.surfaces.covers(crate::slash::catalog::Surface::Tui),
            "{} is not available in the popup's surface",
            entry.name
        );
        assert!(
            !entry.description.is_empty(),
            "{} has no popup description",
            entry.name
        );
    }
    let mut names: Vec<_> = entries.iter().map(|entry| entry.name).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "a catalogued name appears twice");
}

#[test]
fn test_non_slash_line() {
    assert_eq!(slash_candidates("hello", &profiles()), None);
}

#[test]
fn test_slash_only() {
    let (start, end, candidates) = slash_candidates("/", &profiles()).unwrap();
    assert_eq!((start, end), (0, 1));
    assert_eq!(candidates.len(), 61); // registry size: every known command with a description
    // An empty query scores every command equally, so the stable sort keeps
    // registry order and the registry's first entry leads.
    assert_eq!(candidates[0].value, "/start");
    assert_eq!(
        candidates[0].description.as_deref(),
        Some("Start a job for long or background work; ordinary work is just a message")
    );
    // The description is the single-source one from slash::help, sharpened
    // to carry the /connect vs /include contrast (one replaces the
    // active profile, one adds a secondary).
    let connect = candidates
        .iter()
        .find(|candidate| candidate.value == "/connect")
        .expect("connect is registered");
    assert_eq!(
        connect.description.as_deref(),
        Some("Replace the active database profile")
    );
}

#[test]
fn test_command_prefix() {
    let (start, end, candidates) = slash_candidates("/co", &profiles()).unwrap();
    assert_eq!((start, end), (0, 3));
    let values: Vec<_> = candidates.iter().map(|c| c.value.as_str()).collect();
    // "co" prefixes continue, connect, connections, compact, columns,
    // contracts, contract, confirm, context; all tie on score, so the
    // stable sort keeps registry order.
    assert_eq!(
        values,
        vec![
            "/continue",
            "/connect",
            "/connections",
            "/compact",
            "/columns",
            "/contracts",
            "/contract",
            "/confirm",
            "/context",
            "/recovery",
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
            "/continue",
            "/connect",
            "/connections",
            "/compact",
            "/columns",
            "/contracts",
            "/contract",
            "/confirm",
            "/context",
            "/recovery",
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
