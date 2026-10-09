use super::*;

/// Property 2 (consent half): `approvals_are_untouched` — the `ask` prompt
/// keeps its exact bytes and the terminal answers keep their exact meaning
/// with grouping on: the grouper never sees the prompt string (it renders
/// off the `AgentEvent` stream, beside it), and `terminal_choice` maps every
/// answer the same way regardless of what the transcript collapsed. Grouping
/// is presentation over tool events; consent is a different stream.
#[test]
fn ask_prompt_keeps_its_bytes_and_answers_under_grouping() {
    use crate::grant_token::session_answers_line;

    let tool = workspace_write_tool();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let prompt = approval_prompt(
        &tool,
        &arguments,
        Some("workspace-write"),
        &workspace_rooted_facts(),
        None,
        None,
    );
    let answers = session_answers_line(Some("workspace-write"));
    assert_eq!(
        prompt,
        format!(
            "{}\n{answers} ",
            crate::approval_facts::call_facts(
                &tool.name,
                &arguments,
                Some("workspace-write"),
                &workspace_rooted_facts(),
                None,
                None,
            )
            .expect("workspace_write carries facts")
        ),
        "the prompt is the fact body plus the answers line, byte for byte"
    );
    assert!(
        prompt.contains("[a] allow once")
            && prompt.contains("[s] allow workspace-write for this session")
            && prompt.contains("[d] deny"),
        "the prompt keeps its exact prominence: allow, session grant, deny: {prompt:?}"
    );
    assert_eq!(
        terminal_choice("s", Some("workspace-write")),
        ApprovalChoice::AllowSession {
            token: "workspace-write".to_owned()
        },
        "the session-grant answer keeps its meaning"
    );
    assert_eq!(
        terminal_choice("s", None),
        ApprovalChoice::Deny,
        "an unoffered session grant stays a deny"
    );
}
