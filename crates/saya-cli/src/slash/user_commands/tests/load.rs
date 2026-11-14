/// A user command's body is sent as the prompt, with `$ARGUMENTS` replaced by
/// what was typed after the command.
#[test]
fn a_user_command_expands_with_its_arguments() {
    let dir = CommandsDir::new("expand");
    dir.write(
        "review.md",
        "---\ndescription: Review a diff\n---\nReview $ARGUMENTS and report only real problems.\n",
    );
    let commands = load(&dir.0, None);
    let command = commands.get("review").expect("/review loaded");
    assert_eq!(command.description.as_deref(), Some("Review a diff"));
    assert_eq!(command.source, CommandSource::User);
    assert_eq!(
        commands.route("/review src/lib.rs"),
        UserLine::Prompt("Review src/lib.rs and report only real problems.".into())
    );
    assert!(
        commands.notices().is_empty(),
        "a clean load says nothing: {:?}",
        commands.notices()
    );
}

/// A body with no placeholder gets the typed text appended, so `/name foo`
/// never silently drops what was typed.
#[test]
fn arguments_are_appended_when_the_body_has_no_placeholder() {
    let dir = CommandsDir::new("append");
    dir.write("summary.md", "Summarise the working tree.\n");
    let commands = load(&dir.0, None);
    assert_eq!(
        commands.route("/summary since yesterday"),
        UserLine::Prompt("Summarise the working tree.\n\nsince yesterday".into())
    );
    assert_eq!(
        commands.route("/summary"),
        UserLine::Prompt("Summarise the working tree.".into())
    );
}

/// The loader owns only the names it loaded: everything else — a non-slash
/// line, an unknown name, a built-in — is left to the built-in parser.
#[test]
fn lines_the_loader_does_not_own_are_left_to_the_builtin_parser() {
    let dir = CommandsDir::new("builtin");
    dir.write("review.md", "Review $ARGUMENTS.\n");
    let commands = load(&dir.0, None);
    assert_eq!(commands.route("explain the plan"), UserLine::Builtin);
    assert_eq!(commands.route("/unknown"), UserLine::Builtin);
    assert_eq!(commands.route("/help"), UserLine::Builtin);
}

/// A file whose name is a built-in slash command is refused, and the refusal
/// names the built-in it tried to replace.
#[test]
fn a_name_clash_with_a_builtin_is_refused() {
    let dir = CommandsDir::new("clash");
    dir.write("help.md", "You are the help now.\n");
    let commands = load(&dir.0, None);
    assert_eq!(
        commands.route("/help"),
        UserLine::Builtin,
        "the built-in keeps its name"
    );
    let notice = commands
        .notices()
        .iter()
        .find(|notice| notice.contains("help.md"))
        .expect("the refusal names the file");
    assert!(notice.contains("/help is a built-in command"), "{notice}");
    assert!(
        commands
            .ready()
            .all(|command| command.source == CommandSource::Builtin),
        "nothing but the built-ins loaded"
    );
}

/// A file over the per-file byte cap is refused, and typing its name says the
/// cap rather than "unknown command".
#[test]
fn an_oversized_command_file_is_refused() {
    let dir = CommandsDir::new("oversized");
    dir.write("big.md", &"x".repeat(MAX_COMMAND_FILE_BYTES + 1));
    let commands = load(&dir.0, None);
    let refusal = match commands.route("/big") {
        UserLine::Refused(reason) => reason,
        other => panic!("an oversized file is refused, never loaded: {other:?}"),
    };
    assert!(
        refusal.contains(&MAX_COMMAND_FILE_BYTES.to_string()),
        "the refusal names the cap: {refusal}"
    );
    assert!(
        commands
            .notices()
            .iter()
            .any(|notice| notice.contains("big.md")),
        "{:?}",
        commands.notices()
    );
    assert!(
        commands
            .ready()
            .all(|command| command.source == CommandSource::Builtin),
        "nothing but the built-ins loaded"
    );
}

/// A name outside the safe charset is refused: it is not a command name any
/// surface would type.
#[test]
fn a_name_outside_the_safe_charset_is_refused() {
    let dir = CommandsDir::new("charset");
    dir.write("Bad_Name.md", "Do the thing.\n");
    let commands = load(&dir.0, None);
    assert!(
        commands
            .ready()
            .all(|command| command.source == CommandSource::Builtin),
        "nothing but the built-ins loaded"
    );
    assert!(
        commands
            .notices()
            .iter()
            .any(|notice| notice.contains("Bad_Name.md")),
        "{:?}",
        commands.notices()
    );
}

/// A file that loads a name but has no prompt body is refused: an empty
/// message is never sent.
#[test]
fn a_command_without_a_body_is_refused() {
    let dir = CommandsDir::new("empty");
    dir.write("empty.md", "---\ndescription: Nothing\n---\n\n");
    let commands = load(&dir.0, None);
    let refusal = match commands.route("/empty") {
        UserLine::Refused(reason) => reason,
        other => panic!("a bodyless file is refused: {other:?}"),
    };
    assert!(refusal.contains("no prompt body"), "{refusal}");
}

/// A symlinked command file is refused, never read: a project `commands/`
/// entry cannot point the prompt at a file outside the tree.
#[test]
#[cfg(unix)]
fn a_symlinked_command_file_is_refused() {
    use std::os::unix::fs::symlink;
    let dir = CommandsDir::new("symlink");
    let outside = dir.write("outside.txt", "SENTINEL-OUTSIDE-THE-TREE");
    symlink(&outside, dir.0.join("link.md")).expect("the symlink");
    let commands = load(&dir.0, None);
    assert!(
        commands
            .ready()
            .all(|command| command.source == CommandSource::Builtin),
        "nothing but the built-ins loaded"
    );
    match commands.route("/link") {
        UserLine::Refused(reason) => assert!(reason.contains("symlink"), "{reason}"),
        other => panic!("a symlinked file is refused: {other:?}"),
    }
}

/// An expanded prompt over the cap is refused at the point of use.
#[test]
fn an_over_cap_expansion_is_refused() {
    let dir = CommandsDir::new("expansion-cap");
    dir.write("echo.md", "Repeat: $ARGUMENTS\n");
    let commands = load(&dir.0, None);
    let long = "y".repeat(MAX_EXPANDED_PROMPT_BYTES);
    match commands.route(&format!("/echo {long}")) {
        UserLine::Refused(reason) => assert!(reason.contains("over the"), "{reason}"),
        other => panic!("an over-cap expansion is refused: {other:?}"),
    }
}
