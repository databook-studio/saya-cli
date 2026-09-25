use super::tests_support::{idle_app, in_flight_task};
use super::*;

#[test]
fn idle_app_admits_a_sql_command() {
    let app = idle_app();
    assert!(matches!(app.admit_second_sql(), SecondSqlDecision::Start));
    assert!(!app.is_busy(), "idle app is not busy");
}

/// The result-loss root cause: `is_busy()` ignored SQL tasks, so the
/// `!is_busy()` queued-prompt gate stayed open while a query ran and a
/// second command dispatched, replacing `app.sql_task` and dropping the
/// first receiver. With `is_busy()` covering SQL tasks the gate now holds
/// the second command — the first is preserved and reports its result.
#[test]
fn is_busy_covers_a_running_sql_task_so_the_gate_holds_a_second_command() {
    let mut app = idle_app();
    assert!(!app.is_busy(), "idle is not busy");
    app.sql_task = Some(in_flight_task());
    // A running query now reads as busy, so the `!is_busy()` gate will not
    // dispatch a queued line over it.
    assert!(app.is_busy(), "a running SQL task must count as busy");
    // The first task is still tracked — nothing replaced it.
    assert!(app.sql_task.is_some());
}

/// Backstop guard: should a `SqlTask` reach the dispatch handler while one
/// is already running, it is refused (first preserved, message shown) —
/// never a silent replacement.
#[test]
fn second_sql_command_at_the_handler_is_rejected_not_silently_dropped() {
    let mut app = idle_app();
    app.sql_task = Some(in_flight_task());
    assert!(app.is_busy());

    match app.admit_second_sql() {
        SecondSqlDecision::Reject(msg) => {
            assert!(
                msg.to_lowercase().contains("running"),
                "reject message should mention a running command: {msg}"
            );
            assert!(
                !msg.to_lowercase().contains("cancel"),
                "reject must not claim cancellation: {msg}"
            );
        }
        SecondSqlDecision::Start => panic!(
            "a second SQL command at the handler must be rejected, not started (silent result loss)"
        ),
    }
    // Refusing does not drop the running task.
    assert!(app.sql_task.is_some());
}

#[test]
fn detach_sql_task_clears_the_task_and_says_it_is_still_running() {
    let mut app = idle_app();
    app.sql_task = Some(in_flight_task());
    app.detach_sql_task();
    // The UI no longer tracks the query (spinner stops, gate reopens).
    assert!(app.sql_task.is_none(), "detach clears the in-flight task");
    assert!(!app.is_busy());
    // The message must be honest: it is still running server-side, NOT cancelled.
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("a message was posted");
    let text = last.text.to_lowercase();
    assert!(
        text.contains("running"),
        "message says still running: {text}"
    );
    assert!(
        !text.contains("cancel"),
        "detach must not claim cancellation: {text}"
    );
    assert!(
        text.contains("discarded"),
        "message says the result is discarded: {text}"
    );
}

#[test]
fn detach_sql_task_is_a_noop_when_nothing_is_running() {
    let mut app = idle_app();
    app.detach_sql_task();
    assert!(app.sql_task.is_none());
    // No spurious message.
    assert!(app.transcript.blocks().is_empty());
}

/// A running direct-SQL command is visible. The status bar
/// must render a spinner and a "running query" label while a query is in
/// flight, so the user can tell "working" from "hung". Renders through the
/// real `ui::draw` (the same path the snapshot tests use) onto a
/// `TestBackend` and inspects the buffer.
#[test]
fn running_sql_task_is_visible_in_the_status_bar() {
    use super::tests_support::running_app_with_sql_task;
    use crate::interactive::session_prompt::StatusView;

    // The braille spinner frames `ui::panels::SPINNER` cycles through. That
    // constant is private to `ui`, so mirror the frames here to assert one
    // is on screen without reaching across the privacy boundary.
    const SPINNER_CHARS: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let app = running_app_with_sql_task();
    let status = StatusView {
        profile: "analytics".into(),
        included: Vec::new(),
        model: "qwen".into(),
        approval_mode: "read-only".into(),
        agent_mode: "build".into(),
        workspace_root: None,
        sharing_on: true,
        host_composed: false,
        denied_programs: Vec::new(),
        task_summary: None,
    };
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| crate::interactive::tui::ui::draw(frame, &app, &status))
        .expect("draw completes");
    let buffer = format!("{}", terminal.backend());

    // The status bar shows the running-query label (not "thinking") so a
    // SQL task is distinguishable from an agent stream, and a spinner
    // frame is present so the state animates rather than freezing.
    assert!(
        buffer.contains("running query"),
        "status bar should show 'running query' while a SQL task runs:\n{buffer}"
    );
    assert!(
        !buffer.contains("thinking"),
        "a SQL task must not be labelled 'thinking':\n{buffer}"
    );
    assert!(
        SPINNER_CHARS.iter().any(|c| buffer.contains(c)),
        "a spinner frame should be present:\n{buffer}"
    );
}
