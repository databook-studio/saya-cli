/// `/start --tools …` states the session's own narrowing words. The flag is
/// global in this grammar only so it may follow `start`; the parser hands the
/// words to `CodingCommand::Start`, which carries them for the session to
/// narrow with. They are not a CLI argument, so nothing else fills them.
#[test]
fn start_carries_the_session_tools_words() {
    for line in [
        "/start job-a \"a task\" --tools read,grep",
        "/start --tools read,grep job-a \"a task\"",
    ] {
        let Some(SlashCommand::Coding(crate::cli::CodingCommand::Start { tools, .. })) =
            parse_slash_command(line).unwrap()
        else {
            panic!("{line} parses to a start");
        };
        assert_eq!(tools, ["read", "grep"], "{line} states the one list");
    }
}

/// `/start --in-place` states the plain-folder mode on the command the
/// session threads to its worker; omitting it keeps the auto mode.
#[test]
fn slash_start_in_place() {
    let Some(SlashCommand::Coding(crate::cli::CodingCommand::Start { in_place, .. })) =
        parse_slash_command("/start job-a \"a task\" --write . --in-place --approve").unwrap()
    else {
        panic!("the in-place start parses");
    };
    assert!(in_place, "the stated mode is carried");
    let Some(SlashCommand::Coding(crate::cli::CodingCommand::Start { in_place, .. })) =
        parse_slash_command("/start job-a \"a task\" --write . --approve").unwrap()
    else {
        panic!("the plain start parses");
    };
    assert!(!in_place, "no flag, no forced in-place root");
}

/// `/start --managed` states a checkout, and the two launch modes cannot be
/// stated together.
#[test]
fn slash_start_managed() {
    let Some(SlashCommand::Coding(crate::cli::CodingCommand::Start { managed, .. })) =
        parse_slash_command("/start job-a \"a task\" --write . --managed --approve").unwrap()
    else {
        panic!("the managed start parses");
    };
    assert!(managed, "the stated mode is carried");
    let error =
        parse_slash_command("/start job-a \"a task\" --managed --in-place --approve").unwrap_err();
    assert!(
        error.0.contains("--managed") && error.0.contains("--in-place"),
        "the two modes conflict by name: {error}"
    );
}

/// Every other job spelling refuses a stated `--tools` with the one text the
/// CLI's dispatch guard uses, rather than parsing it and dropping it: a
/// session cannot narrow a surface it does not have.
#[test]
fn job_spellings_that_do_not_read_the_flag_refuse_it() {
    for line in [
        "/job resume job-a --tools read",
        "/job show job-a --tools read",
        "/job list --tools read",
        "/follow-up job-a agent-a --expected-completion-sequence 1 \"repair\" --approve --tools read",
        "/approve job-a --tools read",
    ] {
        let error = parse_slash_command(line).unwrap_err();
        assert!(
            error.0.contains("`--tools`"),
            "{line} is refused with the one text: {error}"
        );
    }
}

/// `/job` is the lifecycle command: an agent-shaped verb is refused by name,
/// naming the spelling that works in the session.
#[test]
fn job_refuses_the_agent_shaped_verbs() {
    for (line, spelling) in [
        ("/job approve job-a", "/approve <id>"),
        (
            "/job start job-a \"a task\" --approve",
            "/start <id> <task>",
        ),
        (
            "/job follow-up job-a agent-a --expected-completion-sequence 1 \"x\" --approve",
            "/follow-up <id> <agent>",
        ),
        ("/job cancel-child job-a child-a", "/cancel-child <id> <agent>"),
    ] {
        let error = parse_slash_command(line).unwrap_err();
        assert!(
            error.0.contains(spelling),
            "{line} names {spelling}: {error}"
        );
    }
}

/// The retired `/coding` spelling is still recognized, so its answer can name
/// the spellings that work instead of "unknown command".
#[test]
fn retired_coding_slash_names_the_new_spellings() {
    let error = parse_slash_command("/coding start job-a \"a task\" --approve").unwrap_err();
    for spelling in ["/start", "/follow-up", "/job", "/approve"] {
        assert!(
            error.0.contains(spelling),
            "the retired spelling names {spelling}: {error}"
        );
    }
    assert!(
        !error.0.contains("unknown command"),
        "the retired spelling is answered, not unknown: {error}"
    );
}
