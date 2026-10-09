use super::*;

/// A goal tail rejoins its words into the single positional the headless
/// CLI declares, and passes the flags verbatim — the child's parser stays
/// the authority on them.
#[test]
fn a_goal_tail_becomes_one_positional_then_flags() {
    assert_eq!(
        child_argv("survey the data --allow workspace-write").unwrap(),
        ["survey the data", "--allow", "workspace-write"]
    );
    assert_eq!(child_argv("one goal").unwrap(), ["one goal"]);
    // No goal words at all: an empty positional, which the child's own
    // parser refuses — the adapter does not pre-validate what the child
    // rejects.
    assert_eq!(
        child_argv("--allow workspace-write").unwrap(),
        ["", "--allow", "workspace-write"]
    );
}

#[test]
fn quoted_and_escaped_tail_words_become_literal_argv() {
    assert_eq!(
        child_argv(r#""survey the data" --allow workspace-write"#).unwrap(),
        ["survey the data", "--allow", "workspace-write"]
    );
    assert_eq!(
        child_argv(r#"survey\ the\ data --allow workspace-write"#).unwrap(),
        ["survey the data", "--allow", "workspace-write"]
    );
    assert_eq!(
        child_argv(r#"echo '$(touch pwned)' --allow 'runner:echo'"#).unwrap(),
        ["echo $(touch pwned)", "--allow", "runner:echo"]
    );
    match parse_run_tail(r#""survey the data" --allow workspace-write"#) {
        Ok(RunTail::Start { goal, allow, .. }) => {
            assert_eq!(goal.as_deref(), Some("survey the data"));
            assert_eq!(allow, ["workspace-write"]);
        }
        other => panic!("quoted tail parses to Start, got {other:?}"),
    }
    let tail = match crate::slash::parse_slash_command(
        r#"/run "survey the data" --allow workspace-write"#,
    )
    .unwrap()
    {
        Some(crate::slash::SlashCommand::Run(tail)) => tail,
        other => panic!("quoted slash line parses to Run, got {other:?}"),
    };
    assert!(matches!(parse_run_tail(&tail), Ok(RunTail::Start { .. })));
}

#[test]
fn unmatched_quotes_and_escapes_are_rejected_before_clap() {
    assert!(child_argv(r#""unfinished goal"#).is_err());
    assert!(child_argv("unfinished\\").is_err());
    assert!(parse_run_tail(r#""unfinished goal"#).is_err());
}

/// A subcommand tail passes through verbatim, word by word, so
/// `/run resume <id>` and `/run list` reach the child exactly as typed.
#[test]
fn subcommand_tails_pass_through_verbatim() {
    assert_eq!(child_argv("resume r-1").unwrap(), ["resume", "r-1"]);
    assert_eq!(child_argv("list").unwrap(), ["list"]);
}

/// The panel path parses the tail through the same grammar the child
/// gets: a goal tail becomes one positional plus flags, a management
/// subcommand maps to the shared `RunCommand`, and `resume` is surfaced
/// as its own decline (the resume drive streams to the real stdout).
#[test]
fn the_panel_parses_the_tail_through_the_child_grammar() {
    match parse_run_tail("survey the data --allow workspace-write") {
        Ok(RunTail::Start {
            goal,
            allow,
            budget,
        }) => {
            assert_eq!(goal.as_deref(), Some("survey the data"));
            assert_eq!(allow, vec!["workspace-write".to_string()]);
            assert!(budget.is_empty());
        }
        other => panic!("a goal tail parses to Start, got {other:?}"),
    }
    match parse_run_tail("show r-1") {
        Ok(RunTail::Manage(crate::cli::RunCommand::Show { run_id })) => {
            assert_eq!(run_id, "r-1");
        }
        other => panic!("a show tail parses to Manage, got {other:?}"),
    }
    match parse_run_tail("log r-1") {
        Ok(RunTail::Manage(crate::cli::RunCommand::Log { run_id })) => {
            assert_eq!(run_id, "r-1");
        }
        other => panic!("a log tail parses to Manage, got {other:?}"),
    }
    match parse_run_tail("resume r-1") {
        Ok(RunTail::Resume {
            run_id,
            retry_incomplete: false,
        }) => assert_eq!(run_id, "r-1"),
        other => panic!("a resume tail parses to Resume, got {other:?}"),
    }
    // The parser is the authority: an unknown flag fails the way the
    // child's own parse would. (A budget *value* is validated later, by
    // the run surface — the grammar itself accepts it.)
    assert!(parse_run_tail("--nonsense").is_err());
    match parse_run_tail("--budget nonsense") {
        Ok(RunTail::Start { budget, .. }) => {
            assert_eq!(budget, vec!["nonsense".to_string()]);
        }
        other => panic!("a budget tail parses to Start, got {other:?}"),
    }
}

/// A bypass session's nested `saya run` child gets no `--approval-mode`
/// flag: a run's approval is its `--allow` scopes, and a bypass session's
/// blanket per-call consent never claimed to reach the child. Every other
/// mode is forwarded verbatim, as before.
#[test]
fn a_bypass_session_s_nested_run_child_gets_no_bypass_mode() {
    assert_eq!(forwarded_approval_mode("bypass"), None);
    for (mode, expected) in [
        ("ask", Some("ask")),
        ("read-only", Some("read-only")),
        ("never", Some("never")),
    ] {
        assert_eq!(
            forwarded_approval_mode(mode),
            expected,
            "a {mode} session's child resolves what the session resolved"
        );
    }
    // The guard is the vocabulary's own word, not a prefix match: a mode
    // that merely contains "bypass" is not bypass.
    assert!(
        forwarded_approval_mode("bypassish").is_some(),
        "an unknown mode word is forwarded verbatim, never treated as bypass"
    );
}

/// `--seed-grants` is the slash adapter's own word: it must be the tail's
/// first token, and it never reaches the child. A tail that merely
/// contains it — or a longer flag spelling it — passes through verbatim,
/// and the child's own parser refuses it as the unknown flag it is.
#[test]
fn the_seed_flag_is_the_slash_adapter_s_first_token_only() {
    let (requested, rest) = separate_seed_flag("--seed-grants survey --allow workspace-write");
    assert!(requested, "the leading flag is the adapter's request");
    assert_eq!(
        rest, "survey --allow workspace-write",
        "the tail loses only the flag"
    );
    let (requested, rest) = separate_seed_flag("--seed-grants");
    assert!(
        requested && rest.is_empty(),
        "a bare request seeds an empty tail"
    );
    let (requested, rest) = separate_seed_flag("survey --seed-grants --allow x");
    assert!(
        !requested && rest == "survey --seed-grants --allow x",
        "mid-tail, the word passes to the child verbatim — its parser is the \
         authority and refuses it"
    );
    let (requested, rest) = separate_seed_flag("--seed-grants-only survey");
    assert!(
        !requested && rest == "--seed-grants-only survey",
        "a longer flag spelling is the child's word, never the adapter's request"
    );
    let (requested, rest) = separate_seed_flag("--allow x");
    assert!(!requested && rest == "--allow x");
}
