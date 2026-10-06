//! The empty state under a short terminal: decoration yields before
//! guidance, in an explicit drop order, and the first screen states AI,
//! project, and database readiness independently — an absent database
//! never reads as what stands between the user and their work.
//! Splash-only tests render `draw_empty_state` directly onto a
//! `TestBackend` at an exact pane height; the composed-screen tests go
//! through the real `ui::draw`.

use super::super::splash::{
    DATABASE_NONE, PROJECT_READY, START_AI, START_CODE_BOUND, START_CODE_UNBOUND, START_HEADER,
    START_OPEN, START_QUERY, START_SETUP,
};
use super::*;
use crate::interactive::tui::startup_readiness::{ReadinessLine, StartupReadiness};
use crate::interactive::tui::ui_snapshot_tests::{
    empty_app, empty_app_with_startup, fixed_status, render_buffer,
};

/// The keyboard hint is the last line of the splash, so a clip from the
/// bottom takes it first — the opposite of the drop order.
const HINT: &str = "/ commands     @ tables     ? help     Ctrl+C quit";

/// The concept line the first screen must name once it has room.
const CONCEPT: &str = "Each request becomes a chapter — activity, answer, and results.";

/// The owl's body glyph marks the art: no splash text contains it.
const ART_GLYPH: char = '\u{2588}';

/// An AI line that is not ready, carrying the credential detail the
/// doctor's `ai` task produces for a cloud provider with no key
/// reference.
fn unready_ai() -> StartupReadiness {
    StartupReadiness {
        ai: ReadinessLine {
            ready: false,
            detail: "no api_key reference configured — requests to openai will be unauthenticated"
                .into(),
        },
    }
}

/// Renders the empty state alone at an exact pane size. `profiles` replaces
/// the fixture's demo list; `workspace_bound` drives the workspace paragraph
/// the way `ui::draw` passes it.
fn splash(width: u16, height: u16, profiles: &[&str], workspace_bound: bool) -> String {
    let mut app = empty_app();
    app.profiles = profiles.iter().map(|name| (*name).to_string()).collect();
    paint(&app, width, height, workspace_bound)
}

/// Renders a prepared `App` at an exact pane size, the way the splash helper
/// does, for tests that need state beyond the profile list.
fn paint(app: &App, width: u16, height: u16, workspace_bound: bool) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::draw_empty_state(frame, app, frame.area(), Some(workspace_bound)))
        .expect("draw completes");
    format!("{}", terminal.backend())
}

/// A short terminal clips the splash from the bottom, and the hint is the
/// last line — so the first thing to vanish was how to do anything at all.
/// The drop order must keep the hint to the end.
#[test]
fn a_short_terminal_keeps_the_keyboard_hint() {
    let buffer = splash(80, 14, &[], false);
    assert!(
        buffer.contains(HINT),
        "the hint survives a terminal that cannot fit everything:\n{buffer}"
    );
}

/// The art is pure decoration: it is gone while every line of text still
/// fits, and it stays gone as the squeeze deepens and the state lines are
/// kept.
#[test]
fn a_short_terminal_drops_the_art_first() {
    // Full text fits (with the concept line), compact mascot does not.
    let text_only = splash(80, 25, &[], false);
    assert!(
        !text_only.contains(ART_GLYPH),
        "the art is dropped while every line of text still fits:\n{text_only}"
    );
    assert!(
        text_only.contains("AI        ready") && text_only.contains(HINT),
        "nothing but the art was dropped:\n{text_only}"
    );
    // Deeper squeeze: the art has not come back, the state has not left.
    let squeezed = splash(80, 12, &[], false);
    assert!(
        !squeezed.contains(ART_GLYPH),
        "the art stays gone under a deeper squeeze:\n{squeezed}"
    );
    assert!(
        squeezed.contains("AI        ready") && squeezed.contains(DATABASE_NONE),
        "the state lines outlive the art:\n{squeezed}"
    );
}

/// The three state lines are a stated absence, not chrome: they survive a
/// squeeze that drops the example prompts.
#[test]
fn the_three_state_lines_outlive_the_examples() {
    let buffer = splash(80, 12, &[], true);
    assert!(
        buffer.contains("AI        ready"),
        "the AI readiness line is kept:\n{buffer}"
    );
    assert!(
        buffer.contains(PROJECT_READY),
        "the project readiness line is kept:\n{buffer}"
    );
    assert!(
        buffer.contains(DATABASE_NONE),
        "the database line is kept:\n{buffer}"
    );
    assert!(
        !buffer.contains("try asking"),
        "the examples were dropped by the squeeze:\n{buffer}"
    );
    assert!(buffer.contains(HINT), "the hint is kept too:\n{buffer}");
}

/// The provider-only coding setup — AI ready, a bound workspace, no
/// database — reads as workable: the three readiness lines state their
/// facts independently, and nothing calls the absent database required or
/// sends the user to `config init` for it.
#[test]
fn the_provider_only_setup_never_says_the_database_is_required() {
    let buffer = splash(80, 24, &[], true);
    assert!(
        buffer.contains("AI        ready"),
        "the AI line reads ready:\n{buffer}"
    );
    assert!(
        buffer.contains("Project   ready"),
        "the project line reads ready:\n{buffer}"
    );
    assert!(
        buffer.contains(DATABASE_NONE),
        "the absent database reads as optional:\n{buffer}"
    );
    assert!(
        buffer.contains(START_CODE_BOUND),
        "coding in the bound project is offered:\n{buffer}"
    );
    for absent in [
        "No database is configured",
        "required",
        "Run `saya config init`",
    ] {
        assert!(
            !buffer.contains(absent),
            "`{absent}` must not appear on a screen where coding is ready:\n{buffer}"
        );
    }
}

/// When the provider is not set up, the AI line carries the instruction —
/// the commands that fix it — and the configured databases stay a plain
/// informational line.
#[test]
fn an_unready_provider_names_its_remedy_and_leaves_the_database_alone() {
    let app = empty_app_with_startup(unready_ai());
    let buffer = paint(&app, 80, 30, true);
    assert!(
        buffer.contains("AI        not set up"),
        "the AI line reads not set up:\n{buffer}"
    );
    assert!(
        buffer.contains("no api_key reference configured"),
        "the failing check's detail is kept:\n{buffer}"
    );
    assert!(
        buffer.contains("try /setup, /provider or /model"),
        "the AI line names its remedy:\n{buffer}"
    );
    assert!(
        buffer.contains("Database  analytics  ·  billing"),
        "the configured databases stay informational:\n{buffer}"
    );
}

/// With every need absent, all three lines are present, independent, and
/// each carries its own remedy — nothing says one need depends on another.
#[test]
fn all_three_needs_are_independent_when_everything_is_absent() {
    let mut app = empty_app_with_startup(unready_ai());
    app.profiles.clear();
    let buffer = paint(&app, 80, 30, false);
    assert!(
        buffer.contains("AI        not set up")
            && buffer.contains("try /setup, /provider or /model"),
        "the AI line states its own fact and remedy:\n{buffer}"
    );
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the workspace line states its own fact and remedy ({line:?}):\n{buffer}"
        );
    }
    assert!(
        buffer.contains(DATABASE_NONE),
        "the database line states its own fact and remedy:\n{buffer}"
    );
    assert!(
        !buffer.contains("required"),
        "nothing is presented as required:\n{buffer}"
    );
}

/// The configured-profile path still lists the databases — the drop order
/// must not disturb what the splash says about a present database, and a
/// fully-ready session carries no absence copy at all.
#[test]
fn the_empty_state_is_unchanged_when_a_database_is_configured() {
    let buffer = splash(80, 24, &["analytics", "billing"], true);
    assert!(
        buffer.contains("Database  analytics  ·  billing"),
        "the state line lists the profiles:\n{buffer}"
    );
    assert!(
        !buffer.contains("none yet"),
        "a configured database draws no optional-absence copy:\n{buffer}"
    );
    assert!(buffer.contains("AI        ready"), "the AI line:\n{buffer}");
    assert!(
        buffer.contains("Project   ready"),
        "the project line:\n{buffer}"
    );
    assert!(
        !buffer.contains("No database is configured"),
        "no missing-database guidance:\n{buffer}"
    );
    assert!(buffer.contains(HINT), "the hint:\n{buffer}");
}

/// On a short terminal the entry choices shed first — the three state
/// lines and the hint are what outlive everything.
#[test]
fn short_terminal_drops_the_choices_before_the_state_lines() {
    let buffer = splash(80, 12, &["analytics", "billing"], false);
    assert!(
        buffer.contains("AI        ready"),
        "the AI line survives the squeeze:\n{buffer}"
    );
    assert!(
        buffer.contains("Database  analytics  ·  billing"),
        "the database line survives the squeeze:\n{buffer}"
    );
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the workspace line survives the squeeze ({line:?}):\n{buffer}"
        );
    }
    assert!(
        !buffer.contains(START_HEADER),
        "the choices drop before the state lines:\n{buffer}"
    );
    assert!(
        !buffer.contains("try asking"),
        "the examples drop too:\n{buffer}"
    );
    assert!(buffer.contains(HINT), "the hint survives:\n{buffer}");
}

/// Nothing is dropped when there is room — the fix must not over-trim.
#[test]
fn a_tall_terminal_still_shows_everything() {
    let buffer = splash(80, 40, &[], false);
    assert!(
        buffer.contains(ART_GLYPH),
        "the mascot paints when there is room:\n{buffer}"
    );
    assert!(buffer.contains("◆ saya"), "the splash title:\n{buffer}");
    assert!(
        buffer.contains("Ask your databases in plain language."),
        "the tagline:\n{buffer}"
    );
    assert!(
        buffer.contains("AI        ready"),
        "the AI readiness line:\n{buffer}"
    );
    assert!(
        buffer.contains(DATABASE_NONE),
        "the optional-absence database line:\n{buffer}"
    );
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the workspace line ({line:?}):\n{buffer}"
        );
    }
    assert!(
        buffer.contains(START_HEADER),
        "the choices header:\n{buffer}"
    );
    for choice in [
        START_CODE_UNBOUND,
        START_SETUP,
        START_QUERY,
        START_OPEN,
        START_AI,
    ] {
        assert!(
            buffer.contains(choice),
            "the choice ({choice:?}):\n{buffer}"
        );
    }
    assert!(buffer.contains(CONCEPT), "the concept line:\n{buffer}");
    assert!(
        buffer.contains("try asking"),
        "the examples header:\n{buffer}"
    );
    for prompt in [
        "which tables track billing?",
        "top 5 customers by revenue",
        "compare row counts across the connected databases",
    ] {
        assert!(
            buffer.contains(prompt),
            "the prompt ({prompt:?}):\n{buffer}"
        );
    }
    assert!(buffer.contains(HINT), "the hint:\n{buffer}");
}

/// The entry choices offer the five ways in — in-session guided setup, the
/// sample database, the data file, and the two AI commands — and the
/// dead-end shell pointer the old guidance carried is gone: the first
/// screen points inside the window (U07).
#[test]
fn the_entry_choices_name_exactly_the_five_commands() {
    let buffer = splash(80, 24, &[], true);
    for choice in [
        START_CODE_BOUND,
        START_SETUP,
        START_QUERY,
        START_OPEN,
        START_AI,
    ] {
        assert!(
            buffer.contains(choice),
            "the choice line paints ({choice:?}):\n{buffer}"
        );
    }
    for command in ["/setup", "saya demo", "saya open", "/provider", "/model"] {
        assert!(
            buffer.contains(command),
            "the choices offer `{command}`:\n{buffer}"
        );
    }
    for gone in ["saya setup", "saya config init", "saya connection test"] {
        assert!(
            !buffer.contains(gone),
            "the dead-end command is gone ({gone:?}):\n{buffer}"
        );
    }
}

/// The concept line is copy on the first screen: it must survive the same
/// narrow-terminal rule the guidance copy is held to, or it clips.
#[test]
fn the_concept_line_fits_a_narrow_terminal() {
    let buffer = splash(64, 30, &[], false);
    assert!(
        buffer.contains(CONCEPT),
        "the concept line fits a 64-column terminal:\n{buffer}"
    );
}

/// The absent database's line is the next action: optional, with the two
/// one-command remedies. Pinned by snapshot beside the explicit assertion,
/// so the copy cannot drift silently.
#[test]
fn empty_state_has_next_action() {
    let buffer = splash(80, 30, &[], false);
    assert!(
        buffer.contains(DATABASE_NONE),
        "the absent database reads as optional with two one-command remedies:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}

/// When the session holds starter questions derived from the active
/// profile's cached schema, the examples section shows them instead of the
/// static prompts — the user's own tables beat generic copy. Paint reads
/// the questions from `App` state only; no fetch happens here.
#[test]
fn the_first_screen_asks_about_the_users_own_tables() {
    let mut app = empty_app();
    app.profiles = vec!["demo".into()];
    app.starter_questions = vec![
        "What are the most common channel values in customer_contacts?".to_string(),
        "How does amount_cents change by order_date in orders?".to_string(),
    ];
    let buffer = paint(&app, 80, 30, true);
    assert!(
        buffer.contains("try asking"),
        "the examples header stays:\n{buffer}"
    );
    for question in &app.starter_questions {
        assert!(
            buffer.contains(question.as_str()),
            "the starter question paints ({question:?}):\n{buffer}"
        );
    }
    assert!(
        !buffer.contains("which tables track billing?"),
        "the static prompts yield to the schema-derived ones:\n{buffer}"
    );
    assert!(
        !buffer.contains("top 5 customers by revenue"),
        "the static prompts yield to the schema-derived ones:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}

/// The whole composed frame — context line, splash pane, status bar, input
/// box — keeps the hint when the terminal is short. This is the user-visible
/// shape of the fix, pinned once through the real `ui::draw`.
#[test]
fn the_composed_first_screen_keeps_the_hint_on_a_short_terminal() {
    let mut app = empty_app();
    app.profiles.clear();
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&app, &status, 80, 24);
    assert!(
        buffer.contains(HINT),
        "the composed screen keeps the hint on a short terminal:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}

/// The whole composed frame through the real `ui::draw`, provider-only:
/// AI ready, workspace bound, no database — the coding-ready first screen
/// the old splash misread as broken.
#[test]
fn the_composed_first_screen_reports_all_three_needs() {
    let mut app = empty_app();
    app.profiles.clear();
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("AI        ready"),
        "the AI line reads ready:\n{buffer}"
    );
    assert!(
        buffer.contains("Project   ready"),
        "the project line reads ready:\n{buffer}"
    );
    assert!(
        buffer.contains(DATABASE_NONE),
        "the absent database reads as optional:\n{buffer}"
    );
    assert!(
        buffer.contains(START_CODE_BOUND),
        "coding in the project is offered:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}
