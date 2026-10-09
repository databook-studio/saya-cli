use super::*;

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
