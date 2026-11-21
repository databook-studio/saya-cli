/// `/history` is `/sessions`'s one-release alias: both print the same saved
/// sessions on disk, and the help says so on the alias's side only — the
/// canonical `/sessions` row keeps the picker it opens in the TUI. Both name
/// "saved sessions" so neither implies the conversation or the input-line
/// history.
#[test]
fn history_help_states_the_alias_and_sessions_keeps_its_surfaces() {
    let history = command_help("history").expect("history has help");
    let sessions = command_help("sessions").expect("sessions has help");

    // The alias says what it aliases; the canonical row claims no aliasing.
    assert!(
        history.contains("alias of /sessions"),
        "the alias names its command: {history}"
    );
    assert!(
        !sessions.contains("alias of"),
        "the canonical row claims no aliasing: {sessions}"
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

/// The `/help` listing gives every command line a description: a wall of bare
/// syntax, commands with no explanation, is the defect this test names. A
/// command line is one that introduces a slash command (after the indent); a
/// heading or the title is not a command line and need not carry one.
#[test]
fn listing_gives_every_command_a_description() {
    let summary = help_text();
    // A command line is one that introduces a slash command (after the
    // indent); a heading or the title is not a command line and need not
    // carry a description. A command line "describes" its command when it
    // pairs the usage with prose via the em-dash separator. Today, the
    // listing is a wall of bare syntax — five lines of commands with no
    // description — so this fails until the listing is rebuilt.
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

/// `/context` is `/status`'s one-release alias: the catalogue resolves it, it
/// keeps its own popup description, the listing shows the canonical `/status`
/// row, and its per-command help states its one hard bound: the instruction
/// sources are shown by name, size, digest, and reason — their text is never
/// shown.
#[test]
fn context_has_help_listing_and_description() {
    assert!(
        crate::slash::catalog::resolve("context") == Some("status"),
        "context is /status's alias"
    );
    assert_eq!(
        description_for("context"),
        Some("Deprecated alias for /status (opens the Instructions section; removed next release)"),
        "the alias keeps its own one-line description"
    );
    let listing = help_text();
    assert!(
        listing.contains("/status"),
        "the listing shows the canonical /status row: {listing}"
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
