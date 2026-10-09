//! The host prompt body: the sandbox slot, the no-sandbox pins, and the
//! staged-program integrity line.

use super::host_facts;
use serde_json::json;
use std::path::PathBuf;

/// The trap: a naive port of `run_program` facts would print a sandbox line.
/// The host lane states the absence in the same slot instead.
#[test]
fn the_host_prompt_never_prints_a_sandbox_line() {
    let body = crate::approval_facts::call_facts(
        "run_command",
        &json!({"program": "npm", "args": ["install"]}),
        Some("command:npm"),
        &host_facts(),
        None,
        None,
    )
    .expect("the host lane states its own facts");
    assert!(
        !body.contains("  sandbox:"),
        "the host lane is unsandboxed: no sandbox line, ever: {body}"
    );
    assert!(
        body.contains("  no sandbox:"),
        "the absence is said in the sandbox line's slot: {body}"
    );
}

/// The no-sandbox line's bytes are pinned.
#[test]
fn the_no_sandbox_line_names_user_network_filesystem() {
    let body = crate::approval_facts::call_facts(
        "run_command",
        &json!({"program": "npm", "args": ["install"]}),
        Some("command:npm"),
        &host_facts(),
        None,
        None,
    )
    .expect("the host lane states its own facts");
    for pin in [
        "no sandbox: runs as your user",
        "your whole filesystem",
        "your network",
        "unconfined",
    ] {
        assert!(
            body.contains(pin),
            "the no-sandbox line carries {pin:?}: {body}"
        );
    }
}

/// Once any host command has run, `run_program` gains the integrity line.
#[test]
fn run_program_s_prompt_gains_the_integrity_line_after_any_host_call() {
    let tool = crate::interactive::session_definitions::run_program(saya_agent::ToolDefinition {
        name: "run_program".into(),
        description: String::new(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: saya_agent::ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: saya_agent::LocalStateEffect::WriteWorkspace,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    });
    let arguments = json!({"program": "bench"});
    let mut facts = crate::approval_facts::ApprovalFacts {
        runner: Some(crate::approval_facts::RunnerFacts {
            fs_roots: vec![PathBuf::from("/home/user/proj")],
            runner_programs: vec!["bench".into()],
            ..crate::approval_facts::RunnerFacts::default()
        }),
        ..crate::approval_facts::ApprovalFacts::default()
    };
    let before =
        crate::prompt_approval::approval_prompt(&tool, &arguments, None, &facts, None, None);
    assert!(
        !before.contains("staged program integrity is outside saya's control"),
        "no host call yet: no integrity line: {before}"
    );
    facts.host_ran = true;
    let after =
        crate::prompt_approval::approval_prompt(&tool, &arguments, None, &facts, None, None);
    assert!(
        after.contains(
            "a host command ran in this session; staged program integrity is outside saya's control"
        ),
        "after any host call the integrity line rides: {after}"
    );
}
