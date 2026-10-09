use super::*;

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
