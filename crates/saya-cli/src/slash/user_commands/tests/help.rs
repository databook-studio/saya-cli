/// `/help` lists both kinds of command with their descriptions, beside the
/// built-in listing it already printed.
#[test]
fn help_lists_user_and_project_commands_with_their_descriptions() {
    let user = CommandsDir::new("help-user");
    let project = CommandsDir::new("help-repo");
    user.write(
        "review.md",
        "---\ndescription: Review a diff\n---\nReview $ARGUMENTS.\n",
    );
    project.write(
        "ship.md",
        "---\ndescription: Ship the change\n---\nShip it.\n",
    );
    let commands = load(&user.0, Some(&project.0));
    let listing = crate::slash::help_for(None, &commands);
    assert!(listing.contains("Slash commands:"), "{listing}");
    assert!(listing.contains("/sql"), "the built-ins stand: {listing}");
    assert!(listing.contains("/review — Review a diff"), "{listing}");
    assert!(listing.contains("/ship — Ship the change"), "{listing}");
}

/// `/help <name>` describes one command and names where it came from.
#[test]
fn help_for_one_command_names_its_source() {
    let user = CommandsDir::new("help-one");
    let project = CommandsDir::new("help-one-repo");
    user.write(
        "review.md",
        "---\ndescription: Review a diff\n---\nReview $ARGUMENTS.\n",
    );
    project.write("ship.md", "Ship it.\n");
    let commands = load(&user.0, Some(&project.0));
    let user_help = crate::slash::help_for(Some("review"), &commands);
    assert!(user_help.contains("/review — Review a diff"), "{user_help}");
    assert!(user_help.contains("user command from"), "{user_help}");
    let project_help = crate::slash::help_for(Some("ship"), &commands);
    assert!(project_help.contains("project command from"), "{project_help}");
    assert!(
        crate::slash::help_for(Some("nope"), &commands).contains("No help"),
        "an unknown topic keeps the built-in refusal"
    );
}

/// With no commands directory, `/help` is the built-in listing followed by
/// the shipped default commands and nothing else.
#[test]
fn help_without_a_commands_directory_lists_only_the_defaults() {
    let commands = load(&missing_dir("help-absent"), None);
    let listing = crate::slash::help_for(None, &commands);
    let builtin = crate::slash::help::help_text();
    assert!(
        listing.starts_with(&builtin),
        "the built-in listing still comes first: {listing}"
    );
    let section = &listing[builtin.len()..];
    assert!(section.contains("/commit — "), "{section}");
    assert!(section.contains("/pr — "), "{section}");
    assert_eq!(section.matches("\n  /").count(), 2, "{section}");
}
