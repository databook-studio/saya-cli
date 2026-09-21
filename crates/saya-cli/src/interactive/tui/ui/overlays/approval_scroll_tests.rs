//! Scrolling reaches the withheld facts (E2, audit F05's second half): the
//! whole fact body is reachable at any supported terminal size, and no key
//! used to reach it can ever answer the modal. Real frames through the real
//! `ui::draw`, real key events through the real `handle_key`.

use crate::interactive::tui::keys::handle_key;
use crate::interactive::tui::types::{App, PendingApproval};
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use saya_agent::ApprovalChoice;
use tokio::sync::oneshot::error::TryRecvError;

/// The audit's repro, longer still: a body whose wrapped rows far outrun the
/// capped panel, so the tail of the facts is withheld until scrolled to.
fn long_detail() -> String {
    (1..=30)
        .map(|i| format!("fact line {i:02} of the consequence text"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A second, differently-worded body, so `a_fresh_approval_starts_at_the_top`
/// can tell the two approvals' texts apart in the buffer.
fn other_long_detail() -> String {
    (1..=30)
        .map(|i| format!("second call fact {i:02} of its own consequence"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// An idle app with a pending approval carrying `detail` and `grant`, plus
/// the answer channel a test watches for anything reaching the decider.
fn approval_app(
    detail: Option<String>,
    grant: Option<&str>,
) -> (App, tokio::sync::oneshot::Receiver<ApprovalChoice>) {
    let mut app = empty_app();
    let (respond, answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "http_fetch".into(),
        detail,
        grant: grant.map(str::to_string),
        scroll: 0,
        respond,
    });
    (app, answer)
}

/// The security test, written first: **scrolling never answers.** For each
/// scroll key, with a session grant offered (so an accidental `AllowSession`
/// is possible at all), the modal stays pending and the response channel
/// stays empty. A scroll key that reaches the response channel reintroduces
/// the consent defect slice A fixed for modifiers.
#[test]
fn scrolling_never_answers() {
    for key in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageUp,
        KeyCode::PageDown,
    ] {
        for _ in 0..3 {
            let (mut app, mut answer) =
                approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
            handle_key(&mut app, key, KeyModifiers::NONE);
            assert!(
                app.request.pending_approval.is_some(),
                "{key:?} answered the modal"
            );
            let decision = answer.try_recv();
            assert!(
                matches!(decision, Err(TryRecvError::Empty)),
                "{key:?} reached the response channel: {decision:?}"
            );
        }
    }
}

/// The core test: a body far longer than the panel, scrolled to the end,
/// paints the final fact line. The withheld material is the consequence the
/// user is being asked to authorise; it must be readable.
#[test]
fn the_whole_fact_body_is_reachable_by_scrolling() {
    let (mut app, _answer) = approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
    for _ in 0..40 {
        handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    }
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("fact line 30"),
        "the final fact line is painted after scrolling to the end:\n{buffer}"
    );
}

/// Scrolled into the middle of the body, the answers row still paints: E1
/// reserved its rows out of the panel height, and scrolling moves the detail
/// region only.
#[test]
fn the_answers_stay_pinned_while_scrolled() {
    let (mut app, _answer) = approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
    handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("[a] allow once"),
        "the answers stay painted while scrolled:\n{buffer}"
    );
    assert!(
        buffer.contains("[s] allow fetch:https+api.github.com"),
        "the offered session answer stays painted while scrolled:\n{buffer}"
    );
}

/// Many scrolls in each direction cannot run the offset past either end:
/// clamped at the bottom the final fact paints (nothing blank), clamped at
/// the top the first fact paints, and the answers row stays coherent.
#[test]
fn the_offset_cannot_run_past_either_end() {
    let (mut app, _answer) = approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
    for _ in 0..60 {
        handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    }
    let end_buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        end_buffer.contains("fact line 30"),
        "overscrolling clamps at the end; the last fact paints:\n{end_buffer}"
    );
    assert!(
        end_buffer.contains("[a] allow once"),
        "the answers survive an over-scrolled offset:\n{end_buffer}"
    );
    for _ in 0..60 {
        handle_key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
    }
    let top_buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        top_buffer.contains("fact line 01"),
        "scrolling back past the top clamps; the first fact paints:\n{top_buffer}"
    );
    assert!(
        top_buffer.contains("[a] allow once"),
        "the answers stay painted at the top:\n{top_buffer}"
    );
}

/// The scroll offset is per-pending-approval state that dies with it:
/// scroll one approval, answer it, raise another — the new one shows its
/// first fact, never the previous one's position.
#[test]
fn a_fresh_approval_starts_at_the_top() {
    let (mut app, mut answer) = approval_app(Some(long_detail()), None);
    for _ in 0..5 {
        handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    }
    handle_key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    assert_eq!(
        answer.try_recv().ok(),
        Some(ApprovalChoice::AllowOnce),
        "precondition: the first approval was answered"
    );
    let (respond, _fresh) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "bounded_sql_query".into(),
        detail: Some(other_long_detail()),
        grant: None,
        scroll: 0,
        respond,
    });
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("second call fact 01"),
        "the fresh approval shows its first fact:\n{buffer}"
    );
    assert!(
        !buffer.contains("second call fact 30"),
        "the fresh approval starts at the top, not at the old offset:\n{buffer}"
    );
}

/// A body whose prose lines wrap into several rows at the panel's width:
/// the end-stop counts wrapped rows, so the tail of the last wrapped line
/// is reachable too. A char-count estimate would call this body fitting and
/// strand its tail below the region edge, unlabelled.
#[test]
fn a_wrapped_body_reaches_its_last_wrapped_row() {
    let detail = (1..=6)
        .map(|i| {
            let tail = if i == 6 {
                "zzzz-tail-of-the-facts"
            } else {
                "mid-fact"
            };
            format!(
                "fact {i:02} {} {} {} {}",
                "w".repeat(40),
                "x".repeat(40),
                "y".repeat(40),
                tail
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (mut app, _answer) = approval_app(Some(detail), Some("fetch:https+api.github.com"));
    for _ in 0..40 {
        handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    }
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("zzzz-tail-of-the-facts"),
        "the tail of the last wrapped row is reachable:\n{buffer}"
    );
    assert!(
        buffer.contains("[a] allow once"),
        "the answers survive a wrapped body scrolled to the end:\n{buffer}"
    );
}

/// The control, passing before and after E2: a body that fits does not
/// scroll — the scroll keys leave the fitting panel byte for byte.
#[test]
fn a_body_that_fits_does_not_scroll() {
    let detail = "first fact\nsecond fact\nthird fact".to_string();
    let (mut app, _answer) = approval_app(Some(detail), Some("fetch:https+api.github.com"));
    let before = render_buffer(&app, &fixed_status(), 80, 24);
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    let after = render_buffer(&app, &fixed_status(), 80, 24);
    assert_eq!(before, after, "a fitting body cannot scroll at all");
    assert!(
        !after.contains("more lines hidden"),
        "nothing is withheld when the body fits:\n{after}"
    );
}

/// Slice A's contract, re-asserted because this slice touches that path:
/// every bare answer key still answers exactly as today, and Enter still
/// answers nothing.
#[test]
fn bare_answer_keys_still_answer() {
    for (code, grant, want) in [
        (KeyCode::Char('y'), None, Some(ApprovalChoice::AllowOnce)),
        (KeyCode::Char('a'), None, Some(ApprovalChoice::AllowOnce)),
        (
            KeyCode::Char('s'),
            Some("fetch:https+api.github.com"),
            Some(ApprovalChoice::AllowSession {
                token: "fetch:https+api.github.com".to_owned(),
            }),
        ),
        (KeyCode::Char('n'), None, Some(ApprovalChoice::Deny)),
        (KeyCode::Char('d'), None, Some(ApprovalChoice::Deny)),
        (KeyCode::Esc, None, Some(ApprovalChoice::Deny)),
        (KeyCode::Enter, None, None),
    ] {
        let (mut app, mut answer) = approval_app(Some(long_detail()), grant);
        handle_key(&mut app, code, KeyModifiers::NONE);
        let decision = answer.try_recv();
        assert_eq!(
            decision.ok(),
            want,
            "{code:?} changed its answer under scrolling work"
        );
        match want {
            Some(_) => assert!(
                app.request.pending_approval.is_none(),
                "{code:?} answered but the modal stayed"
            ),
            None => assert!(
                app.request.pending_approval.is_some(),
                "{code:?} answered nothing but the modal left"
            ),
        }
    }
}

/// Slice A's contract again, extended to the new scroll keys: a held
/// Ctrl/Alt/Super answers nothing — and, chosen here, scrolls nothing
/// either. The panel's paint is byte-identical after a modified arrow.
#[test]
fn ctrl_modified_keys_still_answer_nothing() {
    for (code, mods) in [
        (KeyCode::Char('s'), KeyModifiers::CONTROL),
        (KeyCode::Char('n'), KeyModifiers::CONTROL),
        (KeyCode::Down, KeyModifiers::CONTROL),
        (KeyCode::PageDown, KeyModifiers::ALT),
    ] {
        let (mut app, mut answer) =
            approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
        let before = render_buffer(&app, &fixed_status(), 80, 24);
        handle_key(&mut app, code, mods);
        let decision = answer.try_recv();
        assert!(
            matches!(decision, Err(TryRecvError::Empty)),
            "{code:?} with {mods:?} answered the modal: {decision:?}"
        );
        assert!(
            app.request.pending_approval.is_some(),
            "{code:?} with {mods:?} answered the modal"
        );
        let after = render_buffer(&app, &fixed_status(), 80, 24);
        assert_eq!(before, after, "{code:?} with {mods:?} moved the panel");
    }
}
