/// The interpreter/runner split is the run engine's own rule (the grammar
/// mirror at parse time), not an invented one: a name the runner refuses is
/// the interpreter family's member, everything else rides the runner
/// family. (Moved construction, U8: the rule is now asserted through a
/// composition that stages both doors — the family split alone no longer
/// produces a token, the composed membership does.)
#[test]
fn run_program_s_family_rule_is_the_run_engine_s() {
    let both_doors = facts_with_runner(runner_facts(
        &["ripgrep", "ls", "git"],
        &["python3", "python", "bash", "sh", "node", "script"],
    ));
    for (program, family) in [
        ("python3", "interpreter"),
        ("python", "interpreter"),
        ("bash", "interpreter"),
        ("sh", "interpreter"),
        ("node", "interpreter"),
        ("ripgrep", "runner"),
        ("ls", "runner"),
        ("git", "runner"),
        ("script", "interpreter"),
    ] {
        assert_eq!(
            token_for("run_program", json!({"program": program}), &both_doors),
            format!("{family}:{program}"),
            "the family must match the run engine's refusal list"
        );
    }
}

/// The prompt's answers line names the grantable token only when one exists;
/// with none it offers the two answers and says why the third is absent.
#[test]
fn the_answers_line_names_the_token_only_when_one_exists() {
    assert_eq!(
        session_answers_line(Some("workspace-write")),
        "[a] allow once   [s] allow workspace-write for this session   [d] deny",
        "the token is named verbatim — the word the grant records"
    );
    let without = session_answers_line(None);
    assert_eq!(
        without, "[a] allow once   [d] deny   (no session grant for this tool)",
        "with no token the line offers two answers and says so"
    );
}

// --- The composition carries what a token names (U8) --------------------------

/// A token the composition cannot carry is never offered: with
/// `[jobs.interpreter]` empty — the default — a `python3` call suggests no
/// token, so the prompt offers its two answers, never a third that would
/// record a grant pre-answering asks into refusals. The runner door may be
/// fully composed; the interpreter door's universe is what the
/// interpreter family gates on.
#[test]
fn an_unstaged_interpreter_is_never_offered() {
    let runner_composed = facts_with_runner(runner_facts(&["bench"], &[]));
    assert_eq!(
        grant_token(
            "run_program",
            &json!({"program": "python3", "args": ["-c", "print(1)"]}),
            None,
            &runner_composed
        ),
        None,
        "an unstaged interpreter is never offered — `[jobs.interpreter]` \
         carries nothing it could run"
    );
    // And the prompt the two answers produce, not three: no [s] offer at
    // an ask the grant could only pre-answer into a refusal.
    let answers = session_answers_line(None);
    assert!(
        !answers.contains("[s]"),
        "no token, no session-grant offer: {answers}"
    );
    assert!(
        answers.contains("(no session grant for this tool)"),
        "the two-answer line says why the third is absent: {answers}"
    );
}

/// A `runner:` program outside `[jobs.runner] allow` is never offered: the
/// composed allowlist is the door a grant would open, and a program the
/// composition does not carry would refuse at execution — every call, for
/// the rest of the session, with the asks silenced by the grant.
#[test]
fn a_runner_program_outside_the_allowlist_is_never_offered() {
    let runner_composed = facts_with_runner(runner_facts(&["bench"], &[]));
    assert_eq!(
        grant_token(
            "run_program",
            &json!({"program": "deploy"}),
            None,
            &runner_composed
        ),
        None,
        "a program outside the composed [jobs.runner] allow is never offered"
    );
    // No composed runner at all — an unproven host, no workspace root —
    // offers nothing from either family: no run_program call can succeed.
    assert_eq!(
        grant_token(
            "run_program",
            &json!({"program": "bench"}),
            None,
            &ApprovalFacts::default()
        ),
        None,
        "no composed runner, no runner offer at all"
    );
}

/// The write-shaped family gates on the bound workspace root: with no root
/// the tools are not composed, so `workspace-write` names a capability no
/// call could exercise — never offered.
#[test]
fn a_workspace_write_call_with_no_workspace_root_is_never_offered() {
    assert_eq!(
        grant_token(
            "workspace_write",
            &json!({"path": "notes.md", "content": "hello"}),
            None,
            &ApprovalFacts::default()
        ),
        None,
        "no workspace root, no workspace-write offer"
    );
    // Scratch and fetch ride the same rule: a composition without the
    // member offers nothing from it.
    assert_eq!(
        grant_token(
            "scratch_sql",
            &json!({"sql": "SELECT 1"}),
            None,
            &ApprovalFacts::default()
        ),
        None,
        "no scratch member, no scratch offer"
    );
    assert_eq!(
        grant_token(
            "http_fetch",
            &json!({"url": "https://a.example/x"}),
            None,
            &ApprovalFacts::default()
        ),
        None,
        "no fetch member, no fetch offer"
    );
}

/// A composed, staged interpreter **is** still offered — the fix silences
/// only the offer that cannot work, never the honest one. With `python3`
/// staged in `[jobs.interpreter] allow` the call suggests the token the
/// grant records, and an allowlisted runner program its own.
#[test]
fn a_composed_staged_interpreter_is_still_offered() {
    let staged = facts_with_runner(runner_facts(&["bench"], &["python3"]));
    assert_eq!(
        grant_token(
            "run_program",
            &json!({"program": "python3", "args": ["-c", "print(1)"]}),
            None,
            &staged
        ),
        Some("interpreter:python3".to_owned()),
        "a staged interpreter is offered — the honest case must not go silent"
    );
    assert_eq!(
        grant_token("run_program", &json!({"program": "bench"}), None, &staged),
        Some("runner:bench".to_owned()),
        "an allowlisted runner program is offered"
    );
}
