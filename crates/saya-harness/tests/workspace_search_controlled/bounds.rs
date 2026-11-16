use super::super::*;
use saya_harness::workspace::SearchError;

#[test]
fn escaped_output_is_bounded_without_losing_an_exact_completed_count() {
    let fixture = Fixture::new("json-bound");
    let line = format!("needle {}", "\"\\é".repeat(40));
    fixture.put("lines.txt", (line.clone() + "\n").repeat(800).as_bytes());
    let result = fixture.search(
        &query(SearchKind::Grep, "needle", ""),
        SearchLimits {
            result_bytes: 4096,
            ..SearchLimits::default()
        },
    );
    assert!(result.coverage_complete);
    assert_eq!(result.total_matches, Some(800));
    assert_eq!(result.matches_found, 800);
    assert!(result.truncated && !result.matches.is_empty());
    assert!(serde_json::to_vec(&result).unwrap().len() <= 4096);
    assert!(result.matches.len() <= 500);
    for hit in result.matches {
        let SearchMatch::Line { text, .. } = hit else {
            panic!("grep line expected")
        };
        assert_eq!(text, line);
    }
}

#[test]
fn lower_work_bounds_and_skipped_files_never_certify_absence() {
    let fixture = Fixture::new("work-bounds");
    fixture.put("plain.txt", b"observed ordinary bytes\n");
    for limits in [
        SearchLimits {
            entries: 0,
            ..SearchLimits::default()
        },
        SearchLimits {
            read_bytes: 0,
            ..SearchLimits::default()
        },
        SearchLimits {
            file_bytes: 4,
            ..SearchLimits::default()
        },
        SearchLimits {
            timeout: std::time::Duration::ZERO,
            ..SearchLimits::default()
        },
    ] {
        let result = fixture.search(&query(SearchKind::Grep, "missing", ""), limits);
        assert!(!result.coverage_complete && result.truncated);
        assert_eq!(result.total_matches, None);
        assert!(result.matches.is_empty());
        assert!(result.stop_reason.is_some());
    }
    fixture.put("binary.dat", b"\xff\xfe\xfd");
    let result = fixture.search(
        &query(SearchKind::Grep, "missing", ""),
        SearchLimits::default(),
    );
    assert_eq!(result.files_skipped, 1);
    assert!(!result.coverage_complete);
    assert_eq!(result.total_matches, None);
}

/// A result cap too small for the coverage detail is not a refusal: the
/// detail is dropped and the outcome marked, and the answer still fits the
/// cap. Only a cap below the smallest truthful outcome is an invalid limit.
#[test]
fn a_cap_too_small_for_coverage_metadata_drops_it_and_marks_the_outcome() {
    let fixture = Fixture::new("coverage-drop");
    fixture.put("plain.txt", b"observed ordinary bytes\n");
    let plain = query(SearchKind::Grep, "observed", "");
    let token = CancellationToken::new();
    let outcome = fixture
        .workspace
        .search_controlled(
            &plain,
            &SearchLimits {
                result_bytes: MIN_RESULT_BYTES,
                ..SearchLimits::default()
            },
            &token,
        )
        .expect("a cap too small for coverage metadata must still answer");
    assert!(outcome.coverage_dropped);
    assert!(!outcome.coverage_complete);
    assert!(outcome.truncated);
    let encoded = serde_json::to_vec(&outcome).unwrap();
    assert!(
        encoded.len() <= MIN_RESULT_BYTES,
        "the dropped-coverage outcome must fit the cap it was served under: {}",
        encoded.len()
    );
    let served: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(served["coverage_dropped"], true);
    assert!(
        served.get("entries_visited").is_none(),
        "dropped coverage means the counts are absent, not zero: {served}"
    );
    assert!(matches!(
        fixture.workspace.search_controlled(
            &plain,
            &SearchLimits {
                result_bytes: MIN_RESULT_BYTES - 1,
                ..SearchLimits::default()
            },
            &token
        ),
        Err(SearchError::Invalid)
    ));
}

#[test]
fn invalid_widening_regex_paths_and_tiny_caps_are_safe_refusals() {
    let fixture = Fixture::new("invalid");
    let plain = query(SearchKind::Grep, "needle", "");
    let token = CancellationToken::new();
    for limits in [
        SearchLimits {
            entries: 100_001,
            ..SearchLimits::default()
        },
        SearchLimits {
            read_bytes: 64 * 1024 * 1024 + 1,
            ..SearchLimits::default()
        },
        SearchLimits {
            file_bytes: 8 * 1024 * 1024 + 1,
            ..SearchLimits::default()
        },
        SearchLimits {
            matches: 501,
            ..SearchLimits::default()
        },
        SearchLimits {
            line_bytes: 2001,
            ..SearchLimits::default()
        },
        SearchLimits {
            depth: 65,
            ..SearchLimits::default()
        },
        SearchLimits {
            result_bytes: 48 * 1024 + 1,
            ..SearchLimits::default()
        },
        SearchLimits {
            timeout: std::time::Duration::from_secs(3),
            ..SearchLimits::default()
        },
    ] {
        assert!(matches!(
            fixture.workspace.search_controlled(&plain, &limits, &token),
            Err(SearchError::Invalid)
        ));
    }
    for path in ["../outside", "/tmp/anything", "x\\y"] {
        assert!(matches!(
            fixture.workspace.search_controlled(
                &query(SearchKind::Glob, "**", path),
                &SearchLimits::default(),
                &token
            ),
            Err(SearchError::Invalid)
        ));
    }
    // `.git` is ordinary workspace content: a `.git` path is not an argument
    // refusal — whatever the call then returns, it is not Invalid.
    assert!(!matches!(
        fixture.workspace.search_controlled(
            &query(SearchKind::Glob, "**", ".git/config"),
            &SearchLimits::default(),
            &token
        ),
        Err(SearchError::Invalid)
    ));
    for pattern in ["(", "a{1000000}"] {
        let mut regex = query(SearchKind::Grep, pattern, "");
        regex.regex = true;
        assert!(matches!(
            fixture
                .workspace
                .search_controlled(&regex, &SearchLimits::default(), &token),
            Err(SearchError::Invalid)
        ));
    }
    token.cancel();
    assert!(matches!(
        fixture
            .workspace
            .search_controlled(&plain, &SearchLimits::default(), &token),
        Err(SearchError::Cancelled)
    ));
}
