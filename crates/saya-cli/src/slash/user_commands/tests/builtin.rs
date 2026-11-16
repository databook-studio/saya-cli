/// The shipped defaults are ordinary commands: listed with a description,
/// expanded with `$ARGUMENTS`, and routed as a message — no new tool.
#[test]
fn the_builtin_commit_and_pr_commands_are_listed_and_expand() {
    let commands = load(&missing_dir("builtin-none"), None);
    let names: Vec<&str> = commands
        .ready()
        .map(|command| command.name.as_str())
        .collect();
    assert_eq!(names, ["commit", "pr"]);
    assert_eq!(
        commands.get("commit").expect("built in").source,
        CommandSource::Builtin
    );
    assert_eq!(
        commands.get("pr").expect("built in").source,
        CommandSource::Builtin
    );

    match commands.route("/commit fix the parser") {
        UserLine::Prompt(prompt) => {
            assert!(
                prompt.contains("fix the parser") && prompt.contains("conventional-commit"),
                "the hint reaches the commit template: {prompt}"
            );
        }
        other => panic!("the built-in commit command must expand: {other:?}"),
    }
    match commands.route("/pr Add the parser") {
        UserLine::Prompt(prompt) => {
            assert!(
                prompt.contains("Add the parser") && prompt.contains("gh pr create"),
                "the hint reaches the PR template: {prompt}"
            );
        }
        other => panic!("the built-in pr command must expand: {other:?}"),
    }

    let listing = crate::slash::help_for(None, &commands);
    assert!(listing.contains("/commit — "), "{listing}");
    assert!(listing.contains("/pr — "), "{listing}");
}

/// A repository command with a built-in's name replaces it, and the help text
/// states the precedence.
#[test]
fn a_repo_command_overrides_the_builtin_of_the_same_name() {
    let project = CommandsDir::new("builtin-override");
    project.write("commit.md", "Repo commit prompt.\n");
    let commands = load(&missing_dir("builtin-override-user"), Some(&project.0));
    assert_eq!(
        commands.route("/commit"),
        UserLine::Prompt("Repo commit prompt.".into())
    );
    assert_eq!(
        commands.get("commit").expect("the override loads").source,
        CommandSource::Project
    );
    assert!(
        commands
            .notices()
            .iter()
            .any(|notice| notice.contains("replaces the built-in")),
        "the override is said, not silent: {:?}",
        commands.notices()
    );
    let help = crate::slash::help_for(Some("commit"), &commands);
    assert!(help.contains("replaces the built-in"), "{help}");
}

/// The user's own directory overrides a built-in too — precedence is
/// user over built-in, exactly as the help states.
#[test]
fn a_user_command_overrides_the_builtin_of_the_same_name() {
    let user = CommandsDir::new("builtin-user-override");
    user.write("pr.md", "User PR prompt.\n");
    let commands = load(&user.0, None);
    assert_eq!(
        commands.route("/pr"),
        UserLine::Prompt("User PR prompt.".into())
    );
    assert_eq!(
        commands.get("pr").expect("the override loads").source,
        CommandSource::User
    );
}

/// The templates carry the rules the task depends on: no verification bypass,
/// no blanket staging, no co-author trailer; no force-push, and a stated
/// fallback when `gh` is absent.
#[test]
fn the_builtin_templates_carry_the_git_rules() {
    let commands = load(&missing_dir("builtin-rules"), None);
    let commit = &commands.get("commit").expect("built in").body;
    for needle in ["--no-verify", "add -A", "co-author", "secrets"] {
        assert!(commit.contains(needle), "commit template lacks {needle}: {commit}");
    }
    let pr = &commands.get("pr").expect("built in").body;
    for needle in [
        "force-push",
        "gh pr create",
        "compare URL",
        "default branch",
        "permission mode",
    ] {
        assert!(pr.contains(needle), "pr template lacks {needle}: {pr}");
    }
}
