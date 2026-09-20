use super::*;
use crate::interactive::tui::application::tests_support::idle_app;

/// An app with a transcript of distinct matchable lines and the search
/// overlay open in transcript mode for `needle`. The viewport is set wide
/// and tall enough that every line is a wrapped-line of its own.
fn app_with_find(needle: &str) -> App {
    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "alpha one");
    app.transcript
        .push(BlockKind::Assistant, "beta MATCH gamma");
    app.transcript.push(BlockKind::System, "delta two");
    app.transcript.push(BlockKind::Error, "epsilon MATCH zeta");
    app.transcript.push(BlockKind::Tool, "eta three");
    app.viewport.set((40, 10));
    app.open_search(SearchKind::Transcript);
    for c in needle.chars() {
        app.search_char(c);
    }
    app
}

#[test]
fn transcript_overlay_stays_open_after_enter() {
    let mut app = app_with_find("match");
    app.commit_search();
    assert!(
        app.overlays.search.is_some(),
        "Ctrl+F overlay must stay open so Enter can walk matches"
    );
    assert_eq!(
        app.overlays.search.as_ref().unwrap().kind,
        SearchKind::Transcript
    );
}

#[test]
fn repeated_enter_walks_to_distinct_matches() {
    // Before the fix, commit_search took the overlay on every Enter, so
    // there was no find-next — the user retyped the query for each match.
    // Now the overlay stays open and Enter walks to the next match.
    let mut app = app_with_find("match");

    app.commit_search();
    let first = app
        .overlays
        .search
        .as_ref()
        .and_then(|s| s.last_match)
        .expect("first Enter lands on a match");

    app.commit_search();
    let second = app
        .overlays
        .search
        .as_ref()
        .and_then(|s| s.last_match)
        .expect("second Enter lands on a match");

    assert_ne!(
        first, second,
        "repeated Enter must walk to a different match, not re-land on the same one"
    );
}

#[test]
fn editing_the_query_restarts_the_search_from_the_top() {
    let mut app = app_with_find("match");
    app.commit_search(); // lands on the first match
    assert!(app.overlays.search.as_ref().unwrap().last_match.is_some());
    // Typing another char changes the result set; find-next must restart
    // rather than continuing past the now-stale landing.
    app.search_char('x'); // "matchx" — no matches
    assert!(
        app.overlays.search.as_ref().unwrap().last_match.is_none(),
        "an edit resets the last-match cursor"
    );
    app.commit_search();
    // No match for "matchx" → honest message, overlay still open.
    assert!(app.overlays.search.is_some());
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("a message was posted");
    assert!(last.text.contains("no match"), "got: {}", last.text);
}

#[test]
fn no_match_posts_a_message_and_keeps_the_overlay_open() {
    let mut app = app_with_find("nomatch");
    app.commit_search();
    assert!(
        app.overlays.search.is_some(),
        "overlay stays open on a miss"
    );
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("a message was posted");
    assert!(last.text.contains("no match"), "got: {}", last.text);
}

#[test]
fn empty_query_does_not_jump_or_message() {
    let mut app = app_with_find("");
    app.commit_search();
    // Nothing pushed (the app transcript starts empty in idle_app).
    assert!(app.overlays.search.is_some());
    assert!(
        app.overlays.search.as_ref().unwrap().last_match.is_none(),
        "an empty query must not record a landing"
    );
}
