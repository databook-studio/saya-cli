//! Text validation on a record built in code: bounds are counted in bytes,
//! empty or whitespace-only text is refused, and control characters are
//! refused while newline and tab stay allowed.

use super::*;

fn with_text(text: String) -> ProjectPreference {
    ProjectPreference { text, ..record() }
}

// ---------------------------------------------------------------------------
// Text bounds, counted in bytes
// ---------------------------------------------------------------------------

#[test]
fn text_bounds_are_enforced_in_bytes() {
    assert!(
        with_text("a".repeat(MAX_PREFERENCE_TEXT_BYTES))
            .validate()
            .is_ok()
    );
    assert_eq!(
        with_text("a".repeat(MAX_PREFERENCE_TEXT_BYTES + 1))
            .validate()
            .unwrap_err(),
        PreferenceError::TextOversize(MAX_PREFERENCE_TEXT_BYTES + 1)
    );
    // Multi-byte text is counted in bytes, not chars: 512 `é` is 1024 bytes.
    assert!(with_text("é".repeat(512)).validate().is_ok());
    assert!(with_text("é".repeat(513)).validate().is_err());
}

#[test]
fn empty_and_whitespace_text_is_refused() {
    for text in ["", " ", "\t \n"] {
        assert_eq!(
            with_text(text.to_string()).validate().unwrap_err(),
            PreferenceError::TextEmpty
        );
    }
}

#[test]
fn control_characters_are_refused_but_newline_and_tab_are_allowed() {
    assert!(
        with_text("line one\nline two\tindented".to_string())
            .validate()
            .is_ok()
    );
    for text in ["a\u{0}b", "a\u{1}b", "a\rb", "a\u{7}b"] {
        assert_eq!(
            with_text(text.to_string()).validate().unwrap_err(),
            PreferenceError::TextControlCharacter
        );
    }
}
