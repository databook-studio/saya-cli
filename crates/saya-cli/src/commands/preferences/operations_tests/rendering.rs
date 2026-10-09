use super::support::worktree;
use super::*;

/// Rendering: a multi-line, tab-carrying text lists on ONE line (newlines
/// become ` ⏎ `, tabs a space) so the listing stays line-per-preference.
#[test]
fn list_renders_a_multiline_text_on_one_line() {
    let now = now_unix_ms();
    let record = ProjectPreference {
        version: PROJECT_PREFERENCE_VERSION,
        id: PreferenceId::parse("pp-00000000000000000000000000000000").unwrap(),
        worktree: worktree("render"),
        text: "line one\nline two\ttail".to_owned(),
        source: PreferenceSource::UserExplicit,
        created_unix_ms: now - 5_000,
        updated_unix_ms: now,
    };
    let outcome = PreferenceOutcome::Listed {
        worktree_label: worktree("render").as_str().to_owned(),
        items: vec![record],
    };
    let (code, message) = render::render(&outcome);
    assert_eq!(code, 0);
    assert!(
        message.contains("line one ⏎ line two tail"),
        "one line, newline as ⏎, tab as space: {message}"
    );
    assert!(
        message.contains("Project preferences for "),
        "the header names the worktree: {message}"
    );
    assert!(message.contains("(1 of 64)"), "the count header: {message}");
    assert!(
        message.contains(saya_types::MAX_PROJECT_PREFERENCES.to_string().as_str()),
        "the cap is stated: {message}"
    );
    assert!(
        message.contains("They never grant permissions"),
        "the retention sentence rides list: {message}"
    );
}

/// Rendering: an empty listing exits 0 with the add hint (quoted form), and
/// every refusal exits 2.
#[test]
fn empty_list_hints_and_every_refusal_exits_two() {
    let outcome = PreferenceOutcome::Listed {
        worktree_label: "wt-empty".to_owned(),
        items: vec![],
    };
    let (code, message) = render::render(&outcome);
    assert_eq!(code, 0);
    assert!(
        message.contains("No project preferences for wt-empty."),
        "{message}"
    );
    assert!(
        message.contains(concat!("saya preferences add ", "\"", "<text>", "\"")),
        "the add hint quotes the placeholder: {message}"
    );

    for refusal in [
        super::PreferenceRefusal::NoWorktree,
        super::PreferenceRefusal::TextNotAccepted,
        super::PreferenceRefusal::TooMany,
        super::PreferenceRefusal::TooMuchText,
        super::PreferenceRefusal::NotFound,
        super::PreferenceRefusal::InvalidId,
        super::PreferenceRefusal::Unavailable,
    ] {
        let (code, message) = render::render(&PreferenceOutcome::Refused(refusal));
        assert_eq!(code, 2, "{refusal:?} must exit 2, message: {message}");
        assert!(!message.trim().is_empty(), "{refusal:?} states a remedy");
    }

    let (code, message) = render::render(&PreferenceOutcome::Refused(
        super::PreferenceRefusal::NoWorktree,
    ));
    assert_eq!(code, 2);
    assert!(
        message.contains("no project worktree here"),
        "the no-worktree remedy: {message}"
    );
}
