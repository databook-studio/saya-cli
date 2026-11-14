//! The host prompt body: the sandbox slot and the no-sandbox pins.

use super::host_facts;
use serde_json::json;

/// The trap: a naive port of a contained-spawn fact body would print a
/// sandbox line. The host route states the absence in the same slot instead.
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
