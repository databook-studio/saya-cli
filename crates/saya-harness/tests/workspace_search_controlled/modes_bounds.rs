//! The grep options' bounds: what the harness refuses outright, and the
//! ceilings that no new argument may raise. A refused query and an unraised
//! ceiling are the two ways this feature stays bounded, so they are asserted
//! together rather than inferred from the schema.
use super::super::*;
use saya_harness::workspace::{MAX_CONTEXT, SearchError};

fn grep(pattern: &str) -> SearchQuery {
    query(SearchKind::Grep, pattern, "")
}

#[test]
fn undefined_or_unbounded_grep_options_are_refused() {
    let fixture = Fixture::new("refuse-options");
    fixture.put("a.txt", b"needle\n");
    let token = CancellationToken::new();
    let mut cases = Vec::new();
    let mut over_context = grep("needle");
    over_context.grep.context = MAX_CONTEXT + 1;
    cases.push(over_context);
    let mut zero_limit = grep("needle");
    zero_limit.grep.head_limit = Some(0);
    cases.push(zero_limit);
    let mut over_limit = grep("needle");
    over_limit.grep.head_limit = Some(501);
    cases.push(over_limit);
    let mut both = grep("needle");
    both.grep.multiline = true;
    both.grep.context = 1;
    cases.push(both);
    for extensions in [
        vec![],
        vec!["".to_string()],
        vec!["r/s".to_string()],
        vec!["x".repeat(17)],
        (0..9).map(|index| format!("e{index}")).collect(),
    ] {
        let mut bad = grep("needle");
        bad.grep.extensions = Some(extensions);
        cases.push(bad);
    }
    let mut glob_with_grep_options = query(SearchKind::Glob, "**", "");
    glob_with_grep_options.grep = GrepOptions {
        context: 1,
        ..GrepOptions::default()
    };
    cases.push(glob_with_grep_options);
    for case in cases {
        assert!(
            matches!(
                fixture
                    .workspace
                    .search_controlled(&case, &SearchLimits::default(), &token),
                Err(SearchError::Invalid)
            ),
            "{case:?} must be refused"
        );
    }
}

#[test]
fn the_search_ceilings_still_refuse_anything_larger() {
    let fixture = Fixture::new("ceilings");
    fixture.put("a.txt", b"needle\nneedle\n");
    let token = CancellationToken::new();
    let mut at_the_cap = grep("needle");
    at_the_cap.grep.head_limit = Some(500);
    assert!(
        fixture
            .workspace
            .search_controlled(&at_the_cap, &SearchLimits::default(), &token)
            .is_ok(),
        "a head limit at the match cap is answerable"
    );
    for limits in [
        SearchLimits {
            matches: 501,
            ..SearchLimits::default()
        },
        SearchLimits {
            result_bytes: 48 * 1024 + 1,
            ..SearchLimits::default()
        },
        SearchLimits {
            timeout: Duration::from_secs(3),
            ..SearchLimits::default()
        },
    ] {
        assert!(matches!(
            fixture
                .workspace
                .search_controlled(&grep("needle"), &limits, &token),
            Err(SearchError::Invalid)
        ));
    }
}
