// K4: `/continue` is two spellings of two operations, and the six controls are
// `/job`'s verbs under a one-release top-level spelling (row 171).

/// Bare `/continue` is the continuation turn, and it takes no argument.
#[test]
fn a_bare_continue_is_the_continuation_turn() {
    for line in ["/continue", "/continue ", "  /continue  "] {
        assert_eq!(
            parse_slash_command(line).unwrap(),
            Some(SlashCommand::ContinueTurn),
            "{line:?} continues the paused turn"
        );
    }
}

/// `/continue <id> <agent>` is `/job continue <id> <agent>`'s one-release
/// top-level spelling: the same operation, with its alias recorded.
#[test]
fn continue_with_an_id_and_an_agent_is_the_job_verb() {
    let Ok(Some(SlashCommand::Coding(request))) = parse_slash_command("/continue job-a agent-a")
    else {
        panic!("/continue <id> <agent> parses to a job request");
    };
    assert_eq!(
        request.command,
        crate::cli::CodingCommand::Continue {
            id: "job-a".into(),
            agent: "agent-a".into(),
        }
    );
    assert_eq!(request.via_alias, Some(JobAlias::Continue));
    assert!(
        request.note().is_some_and(|note| note.contains("/job continue")),
        "the spelling names the replacement"
    );
}

/// A half-spelled control is a usage error, never read as the continuation
/// turn: `/continue job-a` names one of the two arguments it needs.
#[test]
fn a_half_spelled_continue_is_a_usage_error() {
    let error = parse_slash_command("/continue job-a").unwrap_err();
    assert!(
        error.0.contains("agent"),
        "the refusal names what is missing: {error}"
    );
}

/// The six controls are `/job`'s verbs, each with the alias it stated; the
/// `/job` spellings are the same operations with no alias.
#[test]
fn the_six_controls_are_the_job_verbs_one_release_aliases() {
    for (alias_line, job_line, alias) in [
        ("/approve job-a", "/job approve job-a", JobAlias::Approve),
        ("/deny job-a", "/job deny job-a", JobAlias::Deny),
        (
            "/message job-a agent-a go",
            "/job message job-a agent-a go",
            JobAlias::Message,
        ),
        ("/pause job-a agent-a", "/job pause job-a agent-a", JobAlias::Pause),
        (
            "/cancel-child job-a agent-a",
            "/job cancel-child job-a agent-a",
            JobAlias::CancelChild,
        ),
        (
            "/follow-up job-a agent-a --expected-completion-sequence 1 'go' --approve",
            "/job follow-up job-a agent-a --expected-completion-sequence 1 'go' --approve",
            JobAlias::FollowUp,
        ),
    ] {
        let Ok(Some(SlashCommand::Coding(from_alias))) = parse_slash_command(alias_line) else {
            panic!("{alias_line} parses to a job request");
        };
        let Ok(Some(SlashCommand::Coding(from_job))) = parse_slash_command(job_line) else {
            panic!("{job_line} parses to a job request");
        };
        assert_eq!(
            from_alias.command, from_job.command,
            "{alias_line} and {job_line} are one operation"
        );
        assert_eq!(from_alias.via_alias, Some(alias), "{alias_line} records the alias");
        assert_eq!(from_job.via_alias, None, "{job_line} owes no note");
    }
}

/// `/follow-up` is `/job follow-up` now, and the catalogue says so.
#[test]
fn follow_up_is_the_job_verb_with_a_note() {
    assert_eq!(
        crate::slash::catalog::resolve("follow-up"),
        Some("job"),
        "/follow-up is /job's alias in the one catalogue"
    );
    assert_eq!(
        crate::slash::catalog::resolve("continue"),
        Some("continue"),
        "/continue is its own command: the paused turn"
    );
}
