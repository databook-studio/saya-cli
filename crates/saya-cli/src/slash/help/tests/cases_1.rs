/// `/history` and `/sessions` are **not** interchangeable, and the help
/// must not claim they are. They share one `SessionAction` in the headless
/// REPL, but `tui/dispatch.rs` intercepts the literal line `/sessions`
/// before the parser and opens the interactive picker, so in the TUI —
/// the primary surface — `/sessions` is a picker and `/history` is a text
/// list. Help that called one an alias of the other would be wrong exactly
/// where most people read it. Both name "saved sessions" so neither implies
/// the conversation or the input-line history.
#[test]
fn history_and_sessions_help_describe_their_real_surfaces() {
    let history = command_help("history").expect("history has help");
    let sessions = command_help("sessions").expect("sessions has help");

    // Neither may claim to be an alias of the other: the TUI behaviours differ
    assert!(
        !history.contains("alias") && !sessions.contains("alias"),
        "neither may claim aliasing — the TUI routes them differently: {history} / {sessions}"
    );
    // `/sessions` must mention the picker, which is what the TUI does.
    assert!(
        sessions.contains("picker"),
        "`/sessions` help must mention the picker it opens in the TUI, got: {sessions}"
    );
    // Both are about sessions saved on disk, not the conversation and not
    // the input-line history.
    assert!(
        history.contains("saved") && sessions.contains("saved"),
        "both must name saved sessions, got: {history} / {sessions}"
    );
}

/// transcript; "conversation and context" is accurate and unambiguous.
#[test]
fn clear_help_describes_the_conversation_not_history() {
    let clear = command_help("clear").expect("clear has help");
    assert!(
        clear.contains("conversation") && clear.contains("context"),
        "/clear help should describe the conversation and context, got: {clear}"
    );
    assert!(
        !clear.contains("history"),
        "/clear help must not reuse the overloaded 'history' word (now = saved sessions), got: {clear}"
    );
}

/// Today's `/help` listing is a
/// wall of bare syntax: five lines of commands with no description, and
/// only the last line says what anything does. This test pins that defect
/// by name, so the report can show what changed. It asserts the inverse of
/// what holds today — that every command line in the listing carries a
/// description — and is therefore red until the listing is rebuilt.
#[test]
fn listing_gives_every_command_a_description() {
    let summary = help_text();
    // A command line is one that introduces a slash command (after the
    // indent); a heading or the title is not a command line and need not
    // carry a description. A command line "describes" its command when it
    // pairs the usage with prose via the em-dash separator. Today, the
    // listing is a wall of bare syntax — five lines of commands with no
    // description — so this is red until the listing is rebuilt.
    let bare = summary
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with('/'))
        .filter(|line| !line.contains(" — "))
        .collect::<Vec<_>>();
    assert!(
        bare.is_empty(),
        "every command line should carry a description (— ...), \
         but these are bare syntax with no description:\n{}",
        bare.join("\n")
    );
}

/// `/connect` and `/include` sit beside each other in the
/// listing and must read as a contrast: one replaces the active profile, the
/// other adds a secondary. A user should be able to tell which is which
/// without running `/help connect` and `/help include` separately.
#[test]
fn connect_and_include_read_as_a_contrast() {
    let summary = help_text();

    let connect_line = summary
        .lines()
        .find(|line| line.trim_start().starts_with("/connect "))
        .unwrap_or_else(|| panic!("listing must have a /connect line, got:\n{summary}"));
    let include_line = summary
        .lines()
        .find(|line| line.trim_start().starts_with("/include "))
        .unwrap_or_else(|| panic!("listing must have a /include line, got:\n{summary}"));

    // The two lines must each state their role. "Replace" names the
    // replacing-the-active-profile behaviour of /connect; "secondary" names
    // the adds-a-profile behaviour of /include. Reading both, a user knows
    // one swaps the active profile and the other layers a secondary on.
    assert!(
        connect_line.to_lowercase().contains("replace"),
        "/connect listing must say it replaces the active profile, got: {connect_line}"
    );
    assert!(
        include_line.to_lowercase().contains("secondary"),
        "/include listing must say it adds a secondary profile, got: {include_line}"
    );
}

/// There is one source of description text. The popup in
/// `complete.rs` and the `/help` listing here must not be two hand-maintained
/// copies. The popup reads its descriptions from this module's
/// [`COMMAND_DESCRIPTIONS`]; this test proves that single source covers
/// exactly the parser's registry, so a command added to one surface cannot
/// be missing from the other. (The same agreement is checked from the popup
/// side in `complete.rs::descriptions_cover_exactly_the_registry`, which
/// reads the same shared table.)
#[test]
fn command_descriptions_cover_exactly_the_registry() {
    assert_eq!(
        COMMAND_DESCRIPTIONS.len(),
        registry::KNOWN_COMMANDS.len(),
        "the description table and the command registry must list the same commands"
    );
    for (name, _) in COMMAND_DESCRIPTIONS {
        assert!(
            registry::KNOWN_COMMANDS.contains(name),
            "{name} is described but not in the registry"
        );
    }
    for name in registry::KNOWN_COMMANDS {
        assert!(
            description_for(name).is_some(),
            "{name} is registered but has no description"
        );
    }
}

/// `/context` is registered, listed, described, and has per-command help
/// that states its one hard bound: the instruction sources are shown by
/// name, size, digest, and reason — their text is never shown.
#[test]
fn context_has_help_listing_and_description() {
    assert!(
        registry::KNOWN_COMMANDS.contains(&"context"),
        "context is registered"
    );
    assert!(
        description_for("context").is_some(),
        "context has a popup description"
    );
    let listing = help_text();
    assert!(
        listing.contains("/context"),
        "the listing shows the /context usage: {listing}"
    );
    let help = command_help("context").expect("context has per-command help");
    assert!(
        help.contains("Never their text"),
        "the /context help states what it never shows: {help}"
    );
    assert!(
        help.contains("permissions"),
        "the /context help states the files grant no permissions: {help}"
    );
}
