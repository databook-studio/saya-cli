//! Glob's mtime ordering and its `type` filter: "what did I just change?" is
//! answerable from one call.
use super::super::*;
use saya_harness::workspace::{SearchError, SearchStop};

#[test]
fn glob_serves_the_newest_modified_path_first() {
    let fixture = Fixture::new("glob-order");
    fixture.put_at("old.txt", b"old", at(1_600_000_000));
    fixture.put_at("new.txt", b"new", at(1_600_000_200));
    fixture.put_at("mid.txt", b"mid", at(1_600_000_100));
    let result = fixture.search(&query(SearchKind::Glob, "**", ""), SearchLimits::default());
    assert!(result.coverage_complete, "{result:?}");
    assert_eq!(paths(&result), ["new.txt", "mid.txt", "old.txt"]);
    assert_eq!(result.total_matches, Some(3));
    assert!(!result.truncated);
}

#[test]
fn glob_cap_keeps_the_newest_matches_and_counts_the_rest() {
    let fixture = Fixture::new("glob-order-cap");
    let cap = SearchLimits::default().matches;
    let total = cap + 100;
    for index in 0..total {
        fixture.put_at(
            &format!("f{index:04}.txt"),
            b"",
            at(1_600_000_000 + index as u64),
        );
    }
    let result = fixture.search(
        &query(SearchKind::Glob, "**/*.txt", ""),
        SearchLimits::default(),
    );
    assert!(result.coverage_complete, "{result:?}");
    assert_eq!(result.matches_found, total as u64);
    assert_eq!(result.matches.len(), cap);
    assert_eq!(result.omitted, 100);
    assert!(result.truncated);
    assert_eq!(result.stop_reason, Some(SearchStop::Matches));
    let found = paths(&result);
    assert_eq!(found.first().copied(), Some("f0599.txt"));
    assert_eq!(found.last().copied(), Some("f0100.txt"));
}

/// A time the clock cannot name — a pre-epoch stamp — must not fail the call:
/// those entries fall back to path order, after the ones with a real time.
#[test]
fn glob_without_a_usable_mtime_falls_back_to_path_order() {
    let fixture = Fixture::new("glob-order-fallback");
    let before_epoch = SystemTime::UNIX_EPOCH - Duration::from_secs(60);
    fixture.put_at("z-newest.txt", b"", at(1_600_000_200));
    fixture.put_at("b.txt", b"", before_epoch);
    fixture.put_at("a.txt", b"", before_epoch);
    let result = fixture.search(&query(SearchKind::Glob, "**", ""), SearchLimits::default());
    assert!(result.coverage_complete, "{result:?}");
    assert_eq!(paths(&result), ["z-newest.txt", "a.txt", "b.txt"]);
}

#[test]
fn glob_type_filter_serves_only_the_requested_kind() {
    let fixture = Fixture::new("glob-type");
    fixture.put("src/main.rs", b"needle");
    fixture.put("src/nested/deep.rs", b"needle");
    fixture.put("src/notes.txt", b"needle");
    let files = fixture.search(
        &typed("**/*.rs", SearchEntry::File),
        SearchLimits::default(),
    );
    assert!(files.coverage_complete, "{files:?}");
    assert_eq!(served(&files), ["src/main.rs", "src/nested/deep.rs"]);
    let dirs = fixture.search(&typed("**", SearchEntry::Dir), SearchLimits::default());
    assert!(dirs.coverage_complete, "{dirs:?}");
    assert_eq!(served(&dirs), ["src", "src/nested"]);
    // A file-shaped glob still walks through directories it cannot serve.
    let scoped = fixture.search(
        &typed("src/**/*.txt", SearchEntry::File),
        SearchLimits::default(),
    );
    assert_eq!(served(&scoped), ["src/notes.txt"]);
    // The unfiltered call still serves both kinds.
    let both = fixture.search(&query(SearchKind::Glob, "**", ""), SearchLimits::default());
    assert_eq!(
        served(&both),
        [
            "src",
            "src/main.rs",
            "src/nested",
            "src/nested/deep.rs",
            "src/notes.txt"
        ]
    );
}

/// The filter is a glob concern: a grep query that states one is invalid
/// rather than silently ignored.
#[test]
fn grep_refuses_a_type_filter() {
    let fixture = Fixture::new("glob-type-grep");
    fixture.put("a.txt", b"needle");
    let token = CancellationToken::new();
    let mut grep = query(SearchKind::Grep, "needle", "");
    grep.entry = Some(SearchEntry::File);
    assert!(matches!(
        fixture
            .workspace
            .search_controlled(&grep, &SearchLimits::default(), &token),
        Err(SearchError::Invalid)
    ));
}
