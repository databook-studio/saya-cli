use super::*;

/// The box's title names the running version, so a bug report's screenshot
/// says which build drew it.
#[test]
fn the_title_names_the_version() {
    let buffer = splash(120, 36, &[], true);
    let title = format!("saya v{}", env!("CARGO_PKG_VERSION"));
    assert!(
        buffer.contains(&title),
        "the title names {title:?}:\n{buffer}"
    );
}

/// The two columns carry the welcome, the model/provider/mode line, and
/// both headings.
#[test]
fn the_home_screen_names_its_columns() {
    let buffer = splash(120, 36, &[], true);
    for text in [
        WELCOME,
        "qwen · ollama · read-only",
        GETTING_STARTED,
        STATUS,
    ] {
        assert!(
            buffer.contains(text),
            "the screen paints {text:?}:\n{buffer}"
        );
    }
}

/// With the footer gone, Shift+Tab stays discoverable in the
/// Getting-started list in both workspace states.
#[test]
fn shift_tab_is_named_in_getting_started_at_every_state() {
    for hint in [START_CODE_BOUND, START_CODE_UNBOUND] {
        assert!(
            hint.contains("Shift+Tab"),
            "the Getting-started hint names Shift+Tab: {hint:?}"
        );
    }
}

/// With no model, the left column says the AI is not set up rather than
/// naming a provider it cannot use.
#[test]
fn the_status_line_says_ai_not_set_up_without_a_model() {
    let mut status = fixed_status();
    status.model.clear();
    let buffer = paint_with(&empty_app(), &status, 120, 36, true);
    assert!(
        buffer.contains(AI_NOT_CONFIGURED),
        "the status line names the missing AI:\n{buffer}"
    );
}

/// The Getting-started hints follow the state: a ready AI and a configured
/// database offer how to change them and query, while an unbound, unready,
/// database-less session offers the three setup doors.
#[test]
fn getting_started_chooses_hints_by_state() {
    let ready = splash(120, 36, &["analytics"], true);
    for text in [START_AI, START_SQL, START_CODE_BOUND] {
        assert!(ready.contains(text), "the ready hint {text:?}:\n{ready}");
    }
    for absent in [START_SETUP, START_DATABASE, START_CODE_UNBOUND] {
        assert!(
            !ready.contains(absent),
            "the unready hint {absent:?} is absent:\n{ready}"
        );
    }

    let mut app = empty_app_with_startup(unready_ai());
    app.profiles.clear();
    let unready = paint_with(&app, &fixed_status(), 120, 36, false);
    for text in [START_SETUP, START_DATABASE, START_CODE_UNBOUND] {
        assert!(
            unready.contains(text),
            "the setup hint {text:?}:\n{unready}"
        );
    }
}

/// The Status block keeps the three readiness lines and their wording.
#[test]
fn the_status_block_keeps_the_readiness_lines() {
    let buffer = splash(120, 36, &["analytics", "billing"], true);
    assert!(buffer.contains("AI        ready"), "the AI line:\n{buffer}");
    assert!(
        buffer.contains("Project   ready") && buffer.contains(PROJECT_READY),
        "the project line:\n{buffer}"
    );
    assert!(
        buffer.contains("Database  analytics") && buffer.contains("billing"),
        "the database line:\n{buffer}"
    );
}

/// The unbound-workspace notice paints beside the Status lines, never
/// instead of them.
#[test]
fn the_unbound_notice_paints_beside_the_status_lines() {
    let buffer = splash(120, 36, &[], false);
    for line in NO_WORKSPACE_LINES {
        assert!(buffer.contains(line), "the notice ({line:?}):\n{buffer}");
    }
    assert!(
        buffer.contains(DATABASE_NONE),
        "the database line:\n{buffer}"
    );
}
