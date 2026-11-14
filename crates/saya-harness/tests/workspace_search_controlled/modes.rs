//! Grep's presentation modes: `output_mode`, context lines, a `type`
//! extension filter, `multiline` and `head_limit`. Every mode serves a
//! different entry shape, and every one of them still reports its coverage.
//! What the harness refuses — and the ceilings no argument may raise — live
//! in [`modes_bounds`](super::modes_bounds).
use super::super::*;
use saya_harness::workspace::SearchOutput;

/// The served lines in order, each with whether it is context around a hit.
fn served_lines(outcome: &SearchOutcome) -> Vec<(usize, bool)> {
    outcome
        .matches
        .iter()
        .map(|hit| match hit {
            SearchMatch::Line { line, context, .. } => (*line, *context),
            other => panic!("a content entry is a line: {other:?}"),
        })
        .collect()
}

fn grep(pattern: &str) -> SearchQuery {
    query(SearchKind::Grep, pattern, "")
}

#[test]
fn files_with_matches_serves_paths_and_counts_files() {
    let fixture = Fixture::new("mode-files");
    fixture.put("a.txt", b"needle one\nneedle two\n");
    fixture.put("b.txt", b"needle\n");
    fixture.put("c.txt", b"nothing here\n");
    let mut options = grep("needle");
    options.grep.mode = SearchOutput::FilesWithMatches;
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(
        serde_json::to_value(&result.matches).unwrap(),
        serde_json::json!(["a.txt", "b.txt"]),
        "one bare path per matching file, and no line detail"
    );
    assert_eq!(result.matches_found, 2, "entries are files in this mode");
    assert_eq!(result.total_matches, Some(2));
    assert_eq!(result.omitted, 0);
    assert!(result.coverage_complete && !result.truncated);
}

#[test]
fn count_mode_serves_one_row_per_file_with_its_own_count() {
    let fixture = Fixture::new("mode-count");
    fixture.put("a.txt", b"needle one\nneedle two\n");
    fixture.put("b.txt", b"needle\n");
    fixture.put("c.txt", b"nothing here\n");
    let mut options = grep("needle");
    options.grep.mode = SearchOutput::Count;
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(
        serde_json::to_value(&result.matches).unwrap(),
        serde_json::json!([{"path": "a.txt", "count": 2}, {"path": "b.txt", "count": 1}])
    );
    assert_eq!(result.matches_found, 2, "entries are files in this mode");
    assert_eq!(result.total_matches, Some(2));
    assert_eq!(result.omitted, 0);
    assert!(result.coverage_complete && !result.truncated);
}

#[test]
fn content_mode_keeps_the_line_shape_it_had_before_context_existed() {
    let fixture = Fixture::new("mode-content");
    fixture.put("a.txt", b"needle one\nnothing\n");
    let result = fixture.search(&grep("needle"), SearchLimits::default());
    // A hit's `truncated` is served only when the text was clipped, so a hit
    // that fits carries no flag at all: absence is the default, `false`.
    assert_eq!(
        serde_json::to_value(&result.matches).unwrap(),
        serde_json::json!([{"path": "a.txt", "line": 1, "text": "needle one"}]),
        "a hit carries no context marker and no defaulted truncation flag"
    );
    assert_eq!(result.matches_found, 1);
    assert_eq!(result.total_matches, Some(1));
    assert!(result.coverage_complete);
}

#[test]
fn context_lines_surround_a_hit_at_the_first_and_last_line() {
    let fixture = Fixture::new("context-edges");
    fixture.put("edges.txt", b"needle first\nb\nc\nd\ne\nneedle last\n");
    let mut options = grep("needle");
    options.grep.context = 2;
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(
        served_lines(&result),
        [
            (1, false),
            (2, true),
            (3, true),
            (4, true),
            (5, true),
            (6, false)
        ],
        "two lines each side of both hits, served once each"
    );
    assert_eq!(
        serde_json::to_value(&result.matches[1]).unwrap(),
        serde_json::json!({"path": "edges.txt", "line": 2, "text": "b", "context": true})
    );
    assert_eq!(
        result.matches_found, 2,
        "a context line is served, never counted as found"
    );
    assert_eq!(result.total_matches, Some(2));
    assert_eq!(result.omitted, 0);
    assert!(result.coverage_complete && !result.truncated);
}

#[test]
fn adjacent_hits_do_not_repeat_the_context_between_them() {
    let fixture = Fixture::new("context-adjacent");
    fixture.put("lines.txt", b"a\nneedle\nb\nneedle\nc\n");
    let mut options = grep("needle");
    options.grep.context = 1;
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(
        served_lines(&result),
        [(1, true), (2, false), (3, true), (4, false), (5, true)],
        "the line between two hits is served once"
    );
    assert_eq!(result.matches_found, 2);
    assert!(result.coverage_complete);
}

#[test]
fn head_limit_narrows_the_served_entries_without_lying_about_omitted() {
    let fixture = Fixture::new("head-limit");
    fixture.put("many.txt", b"needle\nneedle\nneedle\nneedle\nneedle\n");
    let mut options = grep("needle");
    options.grep.head_limit = Some(2);
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(
        served_lines(&result),
        [(1, false), (2, false)],
        "only the first two entries are served"
    );
    assert_eq!(result.matches_found, 5);
    assert_eq!(result.total_matches, Some(5));
    assert_eq!(result.omitted, 3);
    assert!(
        result.coverage_complete,
        "the walk still finished: the count is exact, not a guess"
    );
    assert!(result.truncated);
}

#[test]
fn head_limit_narrows_the_files_a_count_mode_serves() {
    let fixture = Fixture::new("head-limit-files");
    fixture.put("a.txt", b"needle\n");
    fixture.put("b.txt", b"needle\n");
    let mut options = grep("needle");
    options.grep.mode = SearchOutput::Count;
    options.grep.head_limit = Some(1);
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches_found, 2);
    assert_eq!(result.total_matches, Some(2));
    assert_eq!(result.omitted, 1);
    assert!(result.coverage_complete && result.truncated);
}

#[test]
fn an_extension_filter_decides_before_the_read() {
    let fixture = Fixture::new("extensions");
    fixture.put("a.rs", b"needle\n");
    fixture.put("b.txt", b"needle\n");
    fixture.put("nested/c.rs", b"needle\n");
    let mut options = grep("needle");
    options.grep.extensions = Some(vec!["rs".into()]);
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(paths(&result), ["a.rs", "nested/c.rs"]);
    assert_eq!(
        result.files_scanned, 2,
        "a file the filter refuses is never read"
    );
    assert_eq!(result.total_matches, Some(2));
    assert!(result.coverage_complete);
}

#[test]
fn multiline_matches_across_a_line_boundary_at_its_start_line() {
    let fixture = Fixture::new("multiline");
    fixture.put("block.txt", b"before\nstart\nmiddle\nend\nafter\n");
    let mut options = grep("start\nmiddle\nend");
    options.grep.multiline = true;
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(
        serde_json::to_value(&result.matches).unwrap(),
        serde_json::json!([{
            "path": "block.txt", "line": 2, "text": "start\nmiddle\nend"
        }])
    );
    assert_eq!(result.matches_found, 1);
    assert!(result.coverage_complete);
    let linewise = fixture.search(&grep("start\nmiddle\nend"), SearchLimits::default());
    assert_eq!(
        linewise.matches_found, 0,
        "the same pattern is no match without multiline"
    );
    assert!(linewise.coverage_complete);
}

#[test]
fn multiline_regex_matches_across_lines_and_stays_bounded() {
    let fixture = Fixture::new("multiline-regex");
    fixture.put("block.txt", b"before\nstart\nmiddle\nend\nafter\n");
    let mut options = grep("(?s)start.*end");
    options.regex = true;
    options.grep.multiline = true;
    let result = fixture.search(&options, SearchLimits::default());
    assert_eq!(result.matches_found, 1);
    assert_eq!(paths(&result), ["block.txt"]);
    assert!(result.coverage_complete);
}
