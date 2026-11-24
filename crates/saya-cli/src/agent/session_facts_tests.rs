//! Twin of `agent/system_prompt_tests/session_facts.rs::session_facts_name_no_tool`,
//! deleted with the old assembler.
//!
//! The live facts section is the bound workspace root; the connections are the
//! database plugin's fact lines. Neither names a tool: the tool schemas are the
//! authority on what is possible, and prose restating them drifts false.

use std::path::Path;

use super::{SESSION_FACTS_HEADING, workspace_root_text};

#[test]
fn the_session_facts_name_no_tool() {
    let text = workspace_root_text(Path::new("/repo"));
    assert!(
        text.starts_with(SESSION_FACTS_HEADING),
        "the facts ride their own heading: {text}"
    );
    assert!(text.contains("/repo"), "the bound root is named: {text}");
    for tool in ["write", "edit", "read", "run_command", "schema_discovery"] {
        assert!(
            !text.contains(tool),
            "the facts section must not name the {tool} tool: {text}"
        );
    }
}
