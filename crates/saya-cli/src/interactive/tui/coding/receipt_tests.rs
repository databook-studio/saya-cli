//! Operator-visible control receipts must survive the progress-event flood:
//! `poll_coding` drains activity first and pushes receipts last, so a
//! receipt is the newest transcript block of its tick and the next frame —
//! which runs before the next drain — always paints it.

use super::{CodingMessage, CodingPanel};
use crate::interactive::tui::types::app::idle_app;
use saya_agent::CancellationToken;
use std::sync::mpsc;

fn panel_with_channels(
    app: &mut crate::interactive::tui::types::App,
    activity: mpsc::Receiver<CodingMessage>,
    receipts: mpsc::Receiver<CodingMessage>,
) -> mpsc::SyncSender<Result<String, String>> {
    let (commands, _commands_rx) = tokio::sync::mpsc::channel(16);
    // The caller keeps the done sender alive so the panel never reports a
    // worker end mid-test.
    let (done_tx, done) = mpsc::sync_channel(1);
    app.coding_panel = Some(CodingPanel {
        id: None,
        rx: activity,
        receipts,
        done,
        commands,
        cancellation: CancellationToken::new(),
        pending: None,
        worker: None,
        active: true,
    });
    done_tx
}

#[test]
fn receipts_are_pushed_after_activity_of_the_same_tick() {
    let mut app = idle_app();
    let (activity_tx, activity_rx) = mpsc::sync_channel::<CodingMessage>(8);
    let (receipt_tx, receipt_rx) = mpsc::sync_channel::<CodingMessage>(8);
    let _done = panel_with_channels(&mut app, activity_rx, receipt_rx);

    for burst in ["progress-1", "progress-2", "progress-3"] {
        activity_tx
            .send(CodingMessage::Activity(burst.into()))
            .unwrap();
    }
    receipt_tx
        .send(CodingMessage::Activity(
            "{\"continue_requested\":true}".into(),
        ))
        .unwrap();

    app.poll_coding();

    let texts: Vec<&str> = app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert!(
        texts.iter().any(|text| text.contains("progress-1")),
        "activity still reaches the transcript: {texts:?}"
    );
    assert_eq!(
        texts.last(),
        Some(&"{\"continue_requested\":true}"),
        "a receipt is the newest block of its tick, so the next frame paints it: {texts:?}"
    );
}

#[test]
fn receipts_surface_even_when_the_activity_queue_is_deep() {
    let mut app = idle_app();
    let (activity_tx, activity_rx) = mpsc::sync_channel::<CodingMessage>(8);
    let (receipt_tx, receipt_rx) = mpsc::sync_channel::<CodingMessage>(8);
    let _done = panel_with_channels(&mut app, activity_rx, receipt_rx);

    for index in 0..8 {
        activity_tx
            .send(CodingMessage::Activity(format!("progress-{index}")))
            .unwrap();
    }
    receipt_tx
        .send(CodingMessage::Activity(
            "{\"cancel_requested\":true}".into(),
        ))
        .unwrap();

    app.poll_coding();

    let texts: Vec<&str> = app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect();
    assert!(
        texts
            .last()
            .is_some_and(|text| text.contains("cancel_requested")),
        "a receipt drains every tick regardless of the activity backlog: {texts:?}"
    );
}

mod safe_text_tests {
    use super::super::safe_text;

    #[test]
    fn terminal_control_text_is_stripped_but_newlines_and_tabs_survive() {
        let hostile = "ok\u{1b}[31mred\u{1b}[0m\u{7}bell\rreturn\u{8}back\nline\ttab";
        let shown = safe_text(hostile);
        assert!(
            !shown
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        );
        assert!(!shown.contains('\u{1b}') && !shown.contains('\u{7}') && !shown.contains('\r'));
        assert!(
            shown.contains("line\ttab") && shown.contains('\n'),
            "{shown:?}"
        );
        assert!(shown.starts_with("ok"), "{shown:?}");
    }

    #[test]
    fn secrets_are_redacted_before_display() {
        let shown = safe_text("connecting with password=hunter2-sentinel done");
        assert!(!shown.contains("hunter2-sentinel"), "{shown}");
        assert!(shown.contains("connecting"), "{shown}");
    }

    #[test]
    fn output_is_bounded_to_sixteen_kibibytes_of_characters() {
        let long = "x".repeat(40 * 1024);
        assert_eq!(safe_text(&long).chars().count(), 16 * 1024);
        let exact = "y".repeat(16 * 1024);
        assert_eq!(
            safe_text(&exact),
            exact,
            "positive control: at the bound nothing is cut"
        );
    }
}
