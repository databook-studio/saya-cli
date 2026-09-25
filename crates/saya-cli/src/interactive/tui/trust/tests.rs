use super::*;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::types::TrustPrompt;

/// `t` trusts the launch cwd: the modal closes and the stashed answer
/// is the canonical cwd — exactly the named directory, never a parent.
#[test]
fn t_trusts_the_launch_cwd() {
    let mut app = idle_app();
    app.overlays.trust = Some(TrustPrompt::default());
    let dir = app
        .answer_trust(KeyCode::Char('t'), KeyModifiers::NONE)
        .expect("t answers");
    assert!(app.overlays.trust.is_none(), "the modal closes");
    assert_eq!(
        app.take_trust_answer().as_deref(),
        Some(dir.as_path()),
        "the answer drains exactly once"
    );
    assert!(
        app.take_trust_answer().is_none(),
        "the drain is single-shot"
    );
    let cwd = std::env::current_dir().unwrap();
    let canonical = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    assert_eq!(dir, canonical, "trust binds exactly the launch cwd");
    let echo = app.transcript.blocks().last().expect("the echo is said");
    assert!(
        echo.text.contains("this session only"),
        "the echo states the session-only shape: {}",
        echo.text
    );
}

/// `c` (and Esc) continue unbound: the modal closes, nothing stashes,
/// nothing binds — today's shape.
#[test]
fn c_and_esc_continue_unbound() {
    for code in [KeyCode::Char('c'), KeyCode::Char('C'), KeyCode::Esc] {
        let mut app = idle_app();
        app.overlays.trust = Some(TrustPrompt::default());
        assert!(
            app.answer_trust(code, KeyModifiers::NONE).is_none(),
            "continue binds nothing: {code:?}"
        );
        assert!(app.overlays.trust.is_none(), "the modal closes: {code:?}");
        assert!(
            app.take_trust_answer().is_none(),
            "nothing stashes for the loop: {code:?}"
        );
    }
}

/// The `w <dir>` line types inline and commits on Enter: the modal
/// closes with exactly the typed directory, canonicalised.
#[test]
fn w_names_a_directory_inline() {
    let mut app = idle_app();
    app.overlays.trust = Some(TrustPrompt::default());
    assert!(
        app.answer_trust(KeyCode::Char('w'), KeyModifiers::NONE)
            .is_none(),
        "w opens the draft, answers nothing"
    );
    assert!(app.overlays.trust.is_some(), "the modal stays while typing");
    let dir = std::env::temp_dir();
    let text = dir.display().to_string();
    for c in text.chars() {
        assert!(
            app.answer_trust(KeyCode::Char(c), KeyModifiers::NONE)
                .is_none(),
            "typing answers nothing"
        );
    }
    let resolved = app
        .answer_trust(KeyCode::Enter, KeyModifiers::NONE)
        .expect("Enter commits the typed dir");
    assert_eq!(
        resolved,
        std::fs::canonicalize(&dir).unwrap(),
        "the modal binds exactly the typed dir"
    );
    assert!(app.overlays.trust.is_none(), "the modal closes");
}

/// A bad directory refuses inline: the modal stays, with the error —
/// never a launch failure, never a silent unbound session.
#[test]
fn a_bad_directory_refuses_inline_and_the_modal_stays() {
    let mut app = idle_app();
    app.overlays.trust = Some(TrustPrompt::default());
    assert!(
        app.answer_trust(KeyCode::Char('w'), KeyModifiers::NONE)
            .is_none()
    );
    for c in "/definitely/not/a/saya/dir".chars() {
        assert!(
            app.answer_trust(KeyCode::Char(c), KeyModifiers::NONE)
                .is_none()
        );
    }
    assert!(
        app.answer_trust(KeyCode::Enter, KeyModifiers::NONE)
            .is_none(),
        "a bad dir answers nothing"
    );
    let prompt = app.overlays.trust.as_ref().expect("the modal stays");
    assert!(
        prompt.error.as_deref().is_some_and(|e| !e.is_empty()),
        "the refusal is said inline"
    );
    assert!(app.take_trust_answer().is_none(), "nothing binds");
}

/// Esc while typing cancels the `w` line back to the modal's first key
/// — it does not continue unbound; a second Esc does that.
#[test]
fn esc_while_typing_cancels_the_draft_first() {
    let mut app = idle_app();
    app.overlays.trust = Some(TrustPrompt::default());
    assert!(
        app.answer_trust(KeyCode::Char('w'), KeyModifiers::NONE)
            .is_none()
    );
    assert!(
        app.answer_trust(KeyCode::Char('x'), KeyModifiers::NONE)
            .is_none()
    );
    assert!(
        app.answer_trust(KeyCode::Esc, KeyModifiers::NONE).is_none(),
        "the first Esc cancels the draft, answers nothing"
    );
    assert!(app.overlays.trust.is_some(), "the modal stays");
    assert!(
        app.answer_trust(KeyCode::Esc, KeyModifiers::NONE).is_none(),
        "the second Esc continues unbound"
    );
    assert!(app.overlays.trust.is_none(), "the modal closes");
}
