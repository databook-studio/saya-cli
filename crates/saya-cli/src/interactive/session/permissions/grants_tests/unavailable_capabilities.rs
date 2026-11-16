/// A token that would gate nothing in this session's composition is
/// refused at `/allow`, with the reason that says why — never seeded: a
/// grant the composition cannot honour would pre-answer asks into refusals and
/// silence every ask that would say something is wrong. Each
/// family's refusal names the token and the composed door it falls outside
/// of; the store keeps exactly what it held. The absent runner and
/// interpreter families are refused in their own words, whatever the
/// composition carries — no fact can carry them back.
#[test]
fn allow_refuses_a_token_the_composition_cannot_carry() {
    // A composition with nothing but the default members refuses the
    // composed families' tokens, each with its own true reason — and the
    // removed families with the removal rule.
    let bare = ApprovalFacts::default();
    let grants = store_with("command:cargo");
    let journal = SessionJournal::open(state_dir("composition-refusal"));
    let cases = [
        ("workspace-write", "no workspace root is bound"),
        ("fetch:https+docs.example", "this session composed no fetch member"),
        ("command:cargo", "the host-command lane is not composed in this session"),
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
    // The removed families: refused even by a composition that carries
    // every other member — their removal is the reason, not a gap.
    for token in ["runner:report", "interpreter:python3"] {
        let error = allow(&[token.to_owned()], &composed_facts(), &grants, &journal)
            .expect_err("the removed families are refused");
        assert!(
            error.contains("runner and interpreter grants were removed"),
            "the refusal states the removal: {error}"
        );
    }
    // The scratch database is gone: the token no longer parses, so it is
    // refused as an unknown scope rather than mapped to a composed door.
    let error = allow(&["scratch".to_owned()], &composed_facts(), &grants, &journal)
        .expect_err("the removed scratch scope is unknown");
    assert!(
        error.contains("unknown scope `scratch`"),
        "the refusal names the removed token: {error}"
    );
    assert_eq!(
        grants.tokens(),
        vec!["command:cargo".to_owned()],
        "a refused /allow seeds nothing — the store keeps exactly what it held"
    );
    assert!(
        journal.read().expect("the journal reads").is_empty(),
        "nothing refused was journalled"
    );
}

/// One `workspace-write` grant pre-answers both guarded file tools: the
/// token `grant_token` yields for a `edit` call is the same word
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
    let token = grant_token("edit", &arguments, None, &facts)
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
