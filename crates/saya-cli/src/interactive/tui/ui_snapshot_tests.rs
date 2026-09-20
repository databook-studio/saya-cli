//! Deterministic snapshots of the **composed** TUI screen — transcript, status
//! bar, and input box drawn together at a real width — so a layout regression
//! is caught in `cargo test` in milliseconds instead of by a human looking at a
//! GIF.
//!
//! These do NOT stand up a store, a database, or a provider. The `App` is built
//! directly as a struct literal (avoiding `App::new`, which reads the history
//! file), the transcript is driven through `stream_events::apply_event` with
//! constructed `AgentEvent`s (the natural seam), and the frame is rendered
//! through the real `ui::draw` onto a `ratatui::backend::TestBackend`. The
//! snapshot is the backend's buffer view (symbols only — colours are stripped),
//! which preserves the trailing whitespace a layout regression would disturb.
//!
//! Three screens, no more — a memory receipt above an answer, a learned plus
//! noted line trailing an answer, and long content at 100×30 proving wrap and
//! truncation. A larger snapshot set gets accepted reflexively, which is the
//! same failure as a weakened assertion.

#[cfg(test)]
#[path = "ui_snapshot_claims.rs"]
mod claims;
#[cfg(test)]
#[path = "ui_snapshot_cursor_labels.rs"]
mod cursor_labels;
#[cfg(test)]
#[path = "ui_snapshot_drafts.rs"]
mod drafts;
#[cfg(test)]
#[path = "ui_snapshot_fold_composer.rs"]
mod fold_composer;
#[cfg(test)]
#[path = "ui_snapshot_gate_action.rs"]
mod gate_action;
#[cfg(test)]
#[path = "ui_snapshot_splash_wide.rs"]
mod splash_wide;
#[cfg(test)]
#[path = "ui_snapshot_support.rs"]
mod support;
#[cfg(test)]
#[path = "ui_snapshot_tables.rs"]
mod tables;

pub(crate) use claims::{proposed_claim, supplied_claim, supplied_contract};
pub(crate) use support::{empty_app, fixed_status, render_buffer, unused_runtime, unused_store};

use super::stream_events::apply_event;
use saya_agent::{AgentEvent, KnowledgeOutcome};
use saya_types::ClaimStatus;

// --- Screen 1: a memory receipt above an answer. ----------------------------

/// A `KnowledgeSupplied` block with 2 claims (one confirmed, one candidate),
/// then the assistant's answer. This is the visibility guarantee: the receipt
/// must not silently lose its shape above the answer.
#[test]
fn memory_receipt_above_answer() {
    let mut app = empty_app();
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![supplied_contract(
                "analytics",
                "catalog.public.orders",
                "current",
                vec![
                    supplied_claim(
                        "ki-abcdef1234",
                        "table_alias",
                        "orders",
                        None,
                        ClaimStatus::Confirmed,
                    ),
                    supplied_claim(
                        "ki-bbbedcafe",
                        "default_time_column",
                        "created_at",
                        Some("created_at"),
                        ClaimStatus::Candidate,
                    ),
                ],
            )],
            0,
        ),
        false,
    );
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("The orders table uses created_at as its time column."),
        false,
    );

    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    insta::assert_snapshot!(buffer);
}

// --- Screen 2: a learned line and a noted line trailing an answer. -----------

/// Assistant text, then a `memory learned · …` line (a confirmed claim) and a
/// `memory noted · … unconfirmed, review with /queue` line (a candidate). Both
/// wordings in one screen, trailing the answer where "and I kept this" belongs.
#[test]
fn learned_and_noted_trailing_answer() {
    let mut app = empty_app();
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("Done — I've recorded what you told me and flagged the guess."),
        false,
    );
    // A user-stated fact lands Confirmed → "learned". Trails the answer.
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_proposed(proposed_claim(
            "ki-learned1",
            "analytics",
            "catalog.public.orders",
            "table_alias",
            "orders",
            None,
            ClaimStatus::Confirmed,
        )),
        false,
    );
    // An assistant inference lands Candidate → "noted", unconfirmed. Trails too.
    apply_event(
        &mut app.transcript,
        AgentEvent::knowledge_proposed(proposed_claim(
            "ki-noted1",
            "analytics",
            "catalog.public.orders",
            "default_time_column",
            "created_at",
            Some("created_at"),
            ClaimStatus::Candidate,
        )),
        false,
    );

    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    insta::assert_snapshot!(buffer);
}

/// Phase 3 packet 2: a folded finished chapter paints as one body row
/// carrying the verbatim request — never a summary — and unfolding restores
/// the painted rows.
#[test]
fn a_folded_chapter_paints_its_request_as_one_row() {
    use super::transcript::BlockKind;
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.transcript.push(BlockKind::User, "and the blue ones");
    let unfolded_rows = app.transcript.total_lines(78);
    assert!(app.transcript.toggle_chapter(1));
    let folded_rows = app.transcript.total_lines(78);
    assert!(
        folded_rows < unfolded_rows,
        "folding removes painted rows ({unfolded_rows} -> {folded_rows})"
    );
    let folded = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        folded.contains("count the red orders"),
        "the folded screen keeps the verbatim request:\n{folded}"
    );
    assert!(
        !folded.contains("the red orders total 42"),
        "the hidden answer leaves the screen:\n{folded}"
    );
    insta::assert_snapshot!(folded);
}

// --- The tool-approval modal renders the shared fact body. -------------------

/// The approval modal renders the per-call fact body verbatim — the same
/// `call_facts` output the terminal prompt renders — plus the shared answers
/// line. This is the modal half of the parity property: the body the modal
/// shows is the body the terminal prompt shows, byte for byte.
#[test]
fn approval_modal_renders_the_shared_fact_body() {
    let mut app = empty_app();
    let tool = crate::interactive::session_definitions::http_fetch();
    let arguments = serde_json::json!({"url": "https://api.github.com/repos/x/y"});
    let facts = crate::approval_facts::ApprovalFacts {
        fetch: Some(crate::approval_facts::FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        ..crate::approval_facts::ApprovalFacts::default()
    };
    let grant = crate::grant_token::grant_token(&tool.name, &arguments, None, &facts);
    let detail = crate::approval_facts::call_facts(
        &tool.name,
        &arguments,
        grant.as_deref(),
        &facts,
        None,
        None,
    );
    let (respond, _answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(super::types::PendingApproval {
        tool: tool.name.clone(),
        detail,
        grant,
        respond,
    });
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    insta::assert_snapshot!(buffer);
}

// --- Fieldnotes phase 6, packet 3: output actions name their scope. ---------
//
// `copy_behaviour_is_unchanged` proves the packet changed words only: Ctrl+Y
// still yields the last assistant block (never a table, never thinking) and
// Ctrl+B still yields the full transcript minus `Thinking`. The help-overlay
// hint test pins that both copy keys name their scope before acting.

/// Ctrl+Y copies the last assistant block; Ctrl+B copies the full transcript
/// minus thinking. No help rewording may change either path.
#[test]
fn copy_behaviour_is_unchanged() {
    use super::transcript::BlockKind;

    // Ctrl+Y: the last assistant block, even with thinking on screen.
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "show the orders");
    app.transcript
        .push(BlockKind::Assistant, "the answer is 42");
    app.transcript
        .push(BlockKind::Thinking, "the secret chain-of-thought");
    app.copy_last_answer();
    assert_eq!(
        app.pending_clipboard.as_deref(),
        Some("the answer is 42"),
        "Ctrl+Y still yields the last assistant block"
    );

    // Ctrl+B: the full untruncated transcript minus thinking.
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "what is the answer");
    app.transcript
        .push(BlockKind::Thinking, "the secret chain-of-thought");
    app.transcript
        .push(BlockKind::Assistant, "the answer is 42");
    app.copy_transcript();
    let copied = app.pending_clipboard.expect("transcript was queued");
    assert!(
        copied.contains("what is the answer") && copied.contains("the answer is 42"),
        "Ctrl+B still yields the full transcript: {copied}"
    );
    assert!(
        !copied.contains("the secret chain-of-thought"),
        "Ctrl+B still leaves thinking out: {copied}"
    );
}

/// The keybinding help overlay names what each copy key acts on: Ctrl+Y the
/// last assistant block (not a table), Ctrl+B the full transcript minus
/// reasoning.
#[test]
fn help_overlay_names_what_each_copy_key_acts_on() {
    let mut app = empty_app();
    app.overlays.show_help = true;
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("last assistant"),
        "the overlay must say Ctrl+Y copies the last assistant block:\n{buffer}"
    );
    assert!(
        buffer.contains("minus reasoning") || buffer.contains("minus thinking"),
        "the overlay must say Ctrl+B leaves reasoning out:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}
