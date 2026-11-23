//! The reading of `@path` mentions: what a message names, what the composer is
//! offering while a mention is typed, and how the list narrows.

use super::{MAX_MENTIONS, accepted, filtered, mentioned, query};

/// A mention is a `@` that starts a word: an address inside a word is not a
/// path, and the same path named twice is carried once.
#[test]
fn a_mention_starts_a_word_and_is_carried_once() {
    let message = "compare @src/a.rs with @src/b.rs, not mail@saya.dev, and @src/a.rs again";
    assert_eq!(mentioned(message), vec!["src/a.rs", "src/b.rs"]);
    assert!(mentioned("no mentions here").is_empty());
    assert!(mentioned("a bare @ names nothing").is_empty());
}

/// One message carries a bounded number of files, whatever it names.
#[test]
fn the_mentions_a_message_carries_are_bounded() {
    let message: String = (0..MAX_MENTIONS + 4)
        .map(|index| format!("@src/f{index}.rs "))
        .collect();
    let named = mentioned(&message);
    assert_eq!(
        named.len(),
        MAX_MENTIONS,
        "the mention bound is the one this module states"
    );
    assert_eq!(named[0], "src/f0.rs", "the first ones are the ones kept");
}

/// The list is open exactly while the composer's end is a mention being typed.
#[test]
fn the_popup_is_open_while_a_mention_is_typed_at_the_end() {
    assert_eq!(query("@"), Some(""));
    assert_eq!(query("look at @src/ma"), Some("src/ma"));
    // A space ends the mention: the path is named, the list is closed.
    assert_eq!(query("look at @src/ma.rs and more"), None);
    // A mention starts a word.
    assert_eq!(query("write to mail@example.com"), None);
    assert_eq!(query(""), None);
}

/// Accepting a row replaces the token being typed, and starts the next word.
#[test]
fn accepting_a_row_replaces_the_typed_token() {
    assert_eq!(
        accepted("look at @src/ma", "src/main.rs"),
        "look at @src/main.rs "
    );
    assert_eq!(accepted("@sr", "src/lib.rs"), "@src/lib.rs ");
    // Nothing is being typed: the line is left exactly as it was.
    assert_eq!(
        accepted("look at @src/a.rs now", "src/b.rs"),
        "look at @src/a.rs now"
    );
}

/// The list is the workspace's paths, fuzzy-matched on the path and bounded.
#[test]
fn the_list_filters_on_the_path_fuzzily() {
    let paths: Vec<String> = ["src/main.rs", "src/lib.rs", "docs/readme.md", "src/mail.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        filtered(&paths, "main", 8),
        vec!["src/main.rs"],
        "a query keeps only the paths it matches"
    );
    let any = filtered(&paths, "", 2);
    assert_eq!(any.len(), 2, "the list is cut to its own row bound");
    assert_eq!(
        filtered(&paths, "zzz", 8),
        Vec::<String>::new(),
        "a query that matches nothing offers nothing"
    );
    let matched = filtered(&paths, "lib", 8);
    assert_eq!(matched, vec!["src/lib.rs"]);
}
