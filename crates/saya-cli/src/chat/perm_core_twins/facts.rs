//! The approval facts on the live shapes: what the operator is told about a
//! `write`, an `edit` and a `shell` call, built from the core definitions
//! rather than from the old session's own advertisement.
//!
//! Twins of `approval_facts_tests::{snapshots::workspace_write_prompt_pins_the_containment_facts,
//! workspace_edit::workspace_edit_facts_state_sizes_never_bodies,
//! host_scope::{a_workspace_write_ask_states_the_grant_covers_every_write,
//! an_ask_with_no_session_grant_offers_no_scope_sentence}}` and of
//! `host_red_tests::grants::interpreter_family_names_add_the_warning_and_command_tokens_never_interpreter_ones`.
//! The facts builder and the scope sentence are the live ones; only the tool
//! the fixture is built from changes.

use std::path::PathBuf;

use saya_agent::WORKSPACE_WRITE_TOKEN;
use serde_json::json;

use super::*;
use crate::approval_facts::{ApprovalFacts, HostFacts, call_facts};
use crate::grant_token::grant_token;

/// A session-shaped facts bundle: a bound workspace root and the host route
/// composed over it — the two members the write and shell families read.
fn session_facts() -> ApprovalFacts {
    ApprovalFacts {
        workspace_root: Some(PathBuf::from("/home/user/proj")),
        host: Some(HostFacts::for_tests()),
        ..ApprovalFacts::default()
    }
}

/// The core `write` ask states the contract the executor enforces —
/// create-if-absent, and a replace only behind the digest the model observed —
/// and never the bytes the model wrote.
#[test]
fn the_core_write_facts_state_the_containment_contract_and_never_the_body() {
    let write = core_file_tool("write");
    assert_eq!(write.name, "write", "the run surface's own file tool");
    let arguments = json!({ "path": "notes/summary.md", "content": "BODY-SECRET-3c8" });
    let facts = call_facts(&write.name, &arguments, None, &session_facts(), None, None)
        .expect("the core write states approval facts");
    assert!(
        !facts.contains("replaced whole"),
        "an existing file is not replaced whole — the ask must not say so: {facts}"
    );
    assert!(
        facts.contains("create-if-absent"),
        "the ask names the create-if-absent contract: {facts}"
    );
    assert!(
        facts.contains("path: notes/summary.md"),
        "the path is stated: {facts}"
    );
    assert!(
        !facts.contains("BODY-SECRET-3c8"),
        "the bytes written never print: {facts}"
    );
}

/// The core `edit` ask states the path and the form's byte sizes, and names
/// the whole-file scope of a `replace_all` — never an anchored body and never
/// the digest.
#[test]
fn the_core_edit_facts_state_sizes_and_the_replace_all_scope_never_bodies() {
    let edit = core_file_tool("edit");
    assert_eq!(edit.name, "edit", "the run surface's own file tool");
    let replace_all = json!({
        "path": "notes/summary.md",
        "old_text": "ANCHOR-OLD-9f3",
        "new_text": "ANCHOR-NEW-7b2",
        "replace_all": true,
        "expected_digest": "sha256:cafe0123456789",
    });
    let facts = call_facts(&edit.name, &replace_all, None, &session_facts(), None, None)
        .expect("a replace_all call has approval facts");
    assert!(
        facts.contains("replace every occurrence of one anchored string"),
        "the ask states the whole-file scope: {facts}"
    );
    assert!(
        facts.contains("old_text 14 bytes") && facts.contains("new_text 14 bytes"),
        "the sizes are stated, never the bodies: {facts}"
    );
    for secret in ["ANCHOR-OLD-9f3", "ANCHOR-NEW-7b2", "sha256:cafe0123456789"] {
        assert!(!facts.contains(secret), "no body or digest text: {facts}");
    }
}

/// The `workspace-write` grant covers every workspace write, and the sentence
/// stating it renders only where a grant is actually on offer: no grant, no
/// scope sentence — never a grant the user is not being offered.
#[test]
fn the_core_write_grant_states_its_scope_and_appears_only_when_offered() {
    let write = core_file_tool("write");
    let arguments = json!({ "path": "notes.md", "content": "hello" });
    let token = grant_token(&write.name, &arguments, None, &session_facts());
    assert_eq!(
        token.as_deref(),
        Some(WORKSPACE_WRITE_TOKEN),
        "the core write's own key is the family token"
    );
    let with = call_facts(
        &write.name,
        &arguments,
        token.as_deref(),
        &session_facts(),
        None,
        None,
    )
    .expect("the write states facts");
    assert!(
        with.contains("covers every workspace write") && with.contains("not this path alone"),
        "the ask names what [s] widens to: {with}"
    );

    let none = call_facts(&write.name, &arguments, None, &session_facts(), None, None)
        .expect("the write states facts");
    for word in ["covers ", "covers every", "any path", "any arguments"] {
        assert!(
            !none.contains(word),
            "no grant offered, no scope sentence — {word} in: {none}"
        );
    }
}

/// The core shell's own key is a command prefix — never an interpreter family's
/// word — and the ask states the host lane it runs in, with no sandbox promise
/// the call cannot keep.
#[test]
fn the_core_shell_key_is_a_prefix_and_the_ask_states_the_host_lane() {
    let shell = core_shell();
    let facts = session_facts();
    for program in ["bash", "python3", "node"] {
        let arguments = json!({ "command": format!("{program} -c x") });
        let token = grant_token(&shell.name, &arguments, None, &facts);
        assert_eq!(
            token,
            Some(format!("shell:{program} -c")),
            "no family mirror: the shell's own prefix is the key"
        );
        let body = call_facts(
            &shell.name,
            &arguments,
            token.as_deref(),
            &facts,
            None,
            None,
        )
        .expect("the shell states its own facts");
        assert!(
            body.contains("runs as your user") && body.contains("unsandboxed"),
            "the ask states the host lane it runs in: {body}"
        );
        assert!(
            !body.contains("no network"),
            "a host call never promises the sandbox's confinement: {body}"
        );
    }
}
