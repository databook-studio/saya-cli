/// One parsed `/start` command, for the grammar tests below.
fn start_command(line: &str) -> crate::cli::CodingCommand {
    let Some(SlashCommand::Coding(command @ crate::cli::CodingCommand::Start { .. })) =
        parse_slash_command(line).unwrap()
    else {
        panic!("{line} parses to a start");
    };
    command
}

/// `/start <task>` takes the whole remaining text as the task and generates
/// the id, so a user types no id to start ordinary work.
#[test]
fn start_generates_an_id_and_takes_the_whole_task() {
    let crate::cli::CodingCommand::Start { id, task, .. } =
        start_command("/start fix the null case in src/lib.rs")
    else {
        unreachable!("start_command returns a start");
    };
    assert_eq!(task, "fix the null case in src/lib.rs");
    saya_types::JobId::parse(&id).expect("the generated id is well formed");
    assert!(!id.is_empty(), "a start always names its job");
}

/// `/start --id <id> <task>` keeps an explicit id available, including the
/// `--id=<id>` spelling.
#[test]
fn start_states_an_explicit_id() {
    for line in ["/start --id job-a fix it", "/start --id=job-a fix it"] {
        let crate::cli::CodingCommand::Start { id, task, .. } = start_command(line) else {
            unreachable!("start_command returns a start");
        };
        assert_eq!(id, "job-a", "{line}");
        assert_eq!(task, "fix it", "{line}");
    }
}

/// Every recognised flag is removed from the task wherever it appears, and
/// the flags keep their own meanings.
#[test]
fn start_removes_recognised_flags_from_the_task() {
    let crate::cli::CodingCommand::Start {
        id,
        task,
        write_paths,
        database_profiles,
        tools,
        plan,
        approve,
        in_place,
        managed,
    } = start_command(
        "/start --write src --database-profile analytics --tools read,grep --plan --approve \
         --id job-a fix the bug --write tests",
    )
    else {
        unreachable!("start_command returns a start");
    };
    assert_eq!(id, "job-a");
    assert_eq!(task, "fix the bug");
    assert_eq!(write_paths, ["src", "tests"]);
    assert_eq!(database_profiles, ["analytics"]);
    assert_eq!(tools, ["read", "grep"]);
    assert!(plan && approve);
    assert!(!in_place && !managed);
}

/// The old positional `/start <id> "<task>"` form is gone: an unquoted first
/// word is ambiguous with a task, so it is task text and the id is generated.
/// Jobs are new in this release, so there is no older spelling to preserve.
#[test]
fn start_treats_the_old_positional_id_as_task_text() {
    let crate::cli::CodingCommand::Start { id, task, .. } =
        start_command("/start job-a \"a task\"")
    else {
        unreachable!("start_command returns a start");
    };
    assert_ne!(id, "job-a", "the first word is never read as an id");
    assert_eq!(task, "job-a a task");
}

/// A start needs a task; flags alone are not work.
#[test]
fn start_requires_a_task() {
    for line in ["/start", "/start --id job-a", "/start --write src"] {
        let error = parse_slash_command(line).unwrap_err();
        assert!(error.0.contains("task"), "{line}: {error}");
    }
}

/// An unrecognised flag before the separator is a mistake worth naming, not
/// silently folded into the task.
#[test]
fn start_refuses_an_unknown_flag() {
    let error = parse_slash_command("/start fix --writ src").unwrap_err();
    assert!(error.0.contains("--writ"), "{error}");
}

/// `--` ends the flags, so task text that starts with dashes stays text.
#[test]
fn start_keeps_dashes_after_the_separator() {
    let crate::cli::CodingCommand::Start { task, .. } =
        start_command("/start explain -- --verbose")
    else {
        unreachable!("start_command returns a start");
    };
    assert_eq!(task, "explain --verbose");
}

/// An invalid or repeated id is refused at parse time, naming the id.
#[test]
fn start_refuses_an_invalid_or_repeated_id() {
    for line in ["/start --id bad/id fix it", "/start --id job-a --id job-b fix it"] {
        let error = parse_slash_command(line).unwrap_err();
        assert!(error.0.contains("id"), "{line}: {error}");
    }
}

/// The two launch modes cannot be stated together, the same conflict the CLI
/// grammar refuses.
#[test]
fn start_refuses_both_launch_modes() {
    let error = parse_slash_command("/start --in-place --managed fix it").unwrap_err();
    assert!(
        error.0.contains("--managed") && error.0.contains("--in-place"),
        "the two modes conflict by name: {error}"
    );
}

/// `/start --tools …` states the session's own narrowing words. The start
/// parser hands the words to `CodingCommand::Start`, which carries them for
/// the session to narrow with. They are not a CLI argument, so nothing else
/// fills them.
#[test]
fn start_carries_the_session_tools_words() {
    for line in [
        "/start --id job-a \"a task\" --tools read,grep",
        "/start --tools read,grep --id job-a \"a task\"",
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
        parse_slash_command("/start --id job-a \"a task\" --write . --in-place --approve").unwrap()
    else {
        panic!("the in-place start parses");
    };
    assert!(in_place, "the stated mode is carried");
    let Some(SlashCommand::Coding(crate::cli::CodingCommand::Start { in_place, .. })) =
        parse_slash_command("/start --id job-a \"a task\" --write . --approve").unwrap()
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
        parse_slash_command("/start --id job-a \"a task\" --write . --managed --approve").unwrap()
    else {
        panic!("the managed start parses");
    };
    assert!(managed, "the stated mode is carried");
    let error =
        parse_slash_command("/start --id job-a \"a task\" --managed --in-place --approve")
            .unwrap_err();
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
        ("/job start job-a \"a task\" --approve", "/start <task>"),
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
