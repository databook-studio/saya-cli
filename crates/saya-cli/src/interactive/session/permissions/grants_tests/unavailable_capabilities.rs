/// A token that would gate nothing in this session's composition is
/// refused at `/allow`, with the reason that says why — never seeded
/// (U8): a grant the composition cannot honour would pre-answer asks into
/// refusals and silence every ask that would say something is wrong. Each
/// family's refusal names the token and the composed door it falls outside
/// of; the store keeps exactly what it held.
#[test]
fn allow_refuses_a_token_the_composition_cannot_carry() {
    // A composition whose runner door carries `bench` only — no staged
    // interpreters, no workspace root — refuses every other family member,
    // each with its own true reason.
    let bare = ApprovalFacts {
        runner: Some(crate::approval_facts::RunnerFacts {
            runner_programs: vec!["bench".into()],
            interpreter_programs: Vec::new(),
            ..crate::approval_facts::RunnerFacts::default()
        }),
        ..ApprovalFacts::default()
    };
    let grants = store_with("runner:bench");
    let journal = SessionJournal::open(state_dir("composition-refusal"));
    let cases = [
        (
            "interpreter:python3",
            "[jobs.interpreter] allow staged no `python3`",
        ),
        (
            "runner:deploy",
            "the composed [jobs.runner] allow carries no `deploy`",
        ),
        ("workspace-write", "no workspace root is bound"),
    ];
    for (token, reason) in cases {
        let error = match allow(&[token.to_owned()], &bare, &grants, &journal) {
            Err(error) => error,
            Ok(message) => panic!(
                "`{token}` gates nothing in this composition and must refuse; it seeded: {message}"
            ),
        };
        assert!(
            error.contains(&format!("scope `{token}`")),
            "the refusal names the token: {error}"
        );
        assert!(
            error.contains(reason),
            "the refusal states its own true reason: {error}"
        );
        assert!(
            error.contains("approving it would gate nothing"),
            "the refusal says what approving would do: {error}"
        );
        assert!(
            error.ends_with("Re-issue /allow without it."),
            "the session tail names the slash surface's next step: {error}"
        );
    }
    assert_eq!(
        grants.tokens(),
        vec!["runner:bench".to_owned()],
        "a refused /allow seeds nothing — the store keeps exactly what it held"
    );
    assert!(
        journal.read().expect("the journal reads").is_empty(),
        "nothing refused was journalled"
    );

    // With the interpreter staged — the composed case — the same token
    // seeds: the refusal is the composition's, never a blanket family ban.
    let facts = composed_facts();
    let message = allow(
        &["interpreter:python3".to_owned()],
        &facts,
        &grants,
        &journal,
    )
    .expect("a staged interpreter is carried, so it seeds");
    assert!(
        message.contains("interpreter:python3"),
        "the carried token seeds: {message}"
    );
    assert!(
        grants.is_granted("interpreter:python3"),
        "the composed case is not refused"
    );
}

/// One `workspace-write` grant pre-answers both guarded file tools: the
/// token `grant_token` yields for a `workspace_edit` call is the same word
/// the store holds, so a WriteWorkspace/requires-approval ask resolves
/// `Allow` without asking. Without the grant the call asks.
#[test]
fn a_workspace_write_grant_pre_answers_workspace_edit() {
    use crate::grant_token::grant_token;
    let facts = ApprovalFacts {
        workspace_root: Some(PathBuf::from("/home/user/proj")),
        ..ApprovalFacts::default()
    };
    let arguments = serde_json::json!({
        "path": "notes.md",
        "old_text": "a",
        "new_text": "b",
        "expected_digest": "d",
    });
    let token = grant_token("workspace_edit", &arguments, None, &facts)
        .expect("the edit suggests the write token");
    assert_eq!(token, "workspace-write", "one token for both file tools");
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    let effect = saya_agent::ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::WriteWorkspace,
    };
    assert_eq!(
        policy.resolve(&effect, Some(&token)),
        saya_agent::ApprovalDecision::Ask,
        "no grant, the call asks"
    );
    policy.grants().grant(&token);
    assert_eq!(
        policy.resolve(&effect, Some(&token)),
        saya_agent::ApprovalDecision::Allow,
        "the one write grant pre-answers the edit"
    );
    assert_eq!(
        policy.resolve(&effect, Some("fetch:https+example.com")),
        saya_agent::ApprovalDecision::Ask,
        "and nothing on another token"
    );
}
