use super::*;

/// A database-only session names its connection and says no workspace is
/// bound — the model must not assume files it cannot see.
#[test]
fn database_only_session_names_connections_and_no_workspace() {
    let reg = single_registry("main");
    let text = session_facts_text(&facts(&reg, &None)).expect("a database session has facts");
    assert!(
        text.contains("main"),
        "the facts must name the connection in scope: {text}"
    );
    assert!(
        text.contains("No workspace is bound"),
        "the facts must say no workspace is bound: {text}"
    );
}

/// A workspace-only session names the root and does not claim a database.
#[test]
fn workspace_only_session_names_root_and_claims_no_database() {
    let empty = ConnectionRegistry::new("main");
    let root = Some(PathBuf::from("/repo"));
    let text = session_facts_text(&facts(&empty, &root)).expect("a workspace session has facts");
    assert!(
        text.contains("/repo"),
        "the facts must name the bound root: {text}"
    );
    assert!(
        text.contains("No database is connected"),
        "the facts must not imply a database is in scope: {text}"
    );
}

/// Both bound: both are named.
#[test]
fn session_with_both_names_both() {
    let reg = multi_registry();
    let root = Some(PathBuf::from("/repo"));
    let text = session_facts_text(&facts(&reg, &root)).expect("a session with both has facts");
    assert!(text.contains("db1"), "both connections named: {text}");
    assert!(text.contains("db2"), "both connections named: {text}");
    assert!(text.contains("/repo"), "the root named: {text}");
}

/// Neither bound: no facts section at all — no empty heading. The legacy
/// no-root entry point is the caller here, so `assemble_system_prompt` keeps
/// a direct production-shaped caller alongside the session-aware paths.
#[test]
fn session_with_neither_produces_no_facts_section() {
    let empty = ConnectionRegistry::new("main");
    assert!(session_facts_text(&facts(&empty, &None)).is_none());
    let prompt = assemble_system_prompt(&empty, MemoryMode::Off, false);
    assert!(
        !prompt
            .as_deref()
            .unwrap_or("")
            .contains(SESSION_FACTS_HEADING),
        "no empty facts heading may appear: {prompt:?}"
    );
}

/// Byte-stability: the cache property, asserted directly.
#[test]
fn session_facts_are_byte_stable_for_the_same_session_inputs() {
    let reg = multi_registry();
    let root = Some(PathBuf::from("/repo"));
    let first =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, false, &facts(&reg, &root));
    let second =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, false, &facts(&reg, &root));
    assert_eq!(
        first, second,
        "same session inputs must produce byte-identical prompts"
    );
}

/// The facts section states facts about the session, never a tool
/// inventory: the tool schemas are the authority on what is possible. (The
/// adjacent multi-connection paragraph names query tools as its navigation
/// instruction; that prose predates this section and is not the facts.)
#[test]
fn session_facts_name_no_tool() {
    for tool in [
        "workspace_write",
        "workspace_edit",
        "workspace_read",
        "run_command",
        "run_program",
        "schema_discovery",
    ] {
        let single = single_registry("main");
        let multi = multi_registry();
        let empty = ConnectionRegistry::new("main");
        let root = Some(PathBuf::from("/repo"));
        for (reg, root) in [(&single, &None), (&multi, &root), (&empty, &root)] {
            let text = session_facts_text(&facts(reg, root)).expect("session facts");
            assert!(
                !text.contains(tool),
                "the facts section must not name the {tool} tool: {text}"
            );
        }
    }
}

/// The session-aware prompt carries the facts. The workspace-less default
/// entry point keeps its prior bytes only insofar as it passes an empty
/// session; a single connection still yields a facts line naming the
/// connection and saying no workspace is bound.
#[test]
fn session_aware_prompt_adds_facts() {
    let reg = single_registry("main");
    let root = Some(PathBuf::from("/repo"));
    let with_session =
        assemble_system_prompt_with_session(&reg, MemoryMode::Off, false, &facts(&reg, &root))
            .expect("a prompt");
    assert!(
        with_session.contains(SESSION_FACTS_HEADING),
        "the session-aware prompt carries the facts: {with_session}"
    );
    assert!(
        with_session.contains("main") && with_session.contains("/repo"),
        "the facts name the connection and the root: {with_session}"
    );
}

/// The working guidance carries the clarification sentence (B3c): a material
/// definition nothing confirmed resolves must be asked about, not assumed.
/// The sentence names the tool the model calls, so prompt and toolset cannot
/// drift apart.
#[test]
fn the_working_guidance_tells_the_model_to_ask_instead_of_assume() {
    let reg = single_registry("main");
    let prompt = assemble_system_prompt(&reg, MemoryMode::Off, false).expect("a prompt");
    assert!(
        prompt.contains("request_clarification"),
        "the guidance names the tool: {prompt}"
    );
    assert!(
        prompt.contains("grain") && prompt.contains("time column") && prompt.contains("metric"),
        "the material definitions are named: {prompt}"
    );
    assert!(
        prompt.contains("ambiguous") && prompt.contains("instead of assuming"),
        "the rule is ask-not-assume: {prompt}"
    );
}
