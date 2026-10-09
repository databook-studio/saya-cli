use super::*;

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
