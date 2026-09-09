//! Terminal approval semantics: what each `ApprovalPolicy` approves through
//! `TerminalApproval` (the decider behind `saya ask` and non-interactive
//! runs). The TUI's `ChannelApproval` (`interactive/tui/agent.rs`) mirrors the
//! read-only rule.

use crate::agent::tools::DatabaseTools;
use crate::prompt_approval::{TerminalApproval, approval_prompt};
use saya_agent::{ApprovalDecider, ApprovalPolicy, LocalStateEffect, ToolDefinition, ToolEffect};

fn side_effecting_tool() -> ToolDefinition {
    ToolDefinition {
        name: "run_program".into(),
        description: "spawns a process outside the agent".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
    }
}

fn database_tools() -> Vec<ToolDefinition> {
    DatabaseTools::definitions(true, false, false)
}

#[tokio::test]
async fn read_only_approval_denies_a_side_effecting_tool() {
    let approval = TerminalApproval::new(ApprovalPolicy::ReadOnly, false);
    assert!(
        !approval
            .approve(&side_effecting_tool(), &serde_json::json!({}))
            .await,
        "read-only must not auto-approve a tool with an external side effect"
    );
}

#[tokio::test]
async fn non_interactive_read_only_denies_render_chart_and_still_approves_sql() {
    let approval = TerminalApproval::new(ApprovalPolicy::ReadOnly, false);
    let tools = database_tools();
    let render_chart = tools
        .iter()
        .find(|tool| tool.name == "render_chart")
        .expect("render_chart is defined");
    assert!(
        !approval
            .approve(
                render_chart,
                &serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"})
            )
            .await,
        "render_chart writes a file and spawns a browser; read-only must deny it"
    );
    let sql = tools
        .iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    assert!(
        approval
            .approve(sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await,
        "the SQL tools must stay auto-approved under read-only"
    );
}

/// The arms the read-only change must not touch: `never` denies, and `ask`
/// without a terminal to prompt on denies — no matter how read-shaped the tool
/// is.
#[tokio::test]
async fn never_denies_and_ask_without_a_terminal_denies() {
    let tools = database_tools();
    let sql = tools
        .iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    let never = TerminalApproval::new(ApprovalPolicy::Never, false);
    assert!(
        !never
            .approve(sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await
    );
    let ask = TerminalApproval::new(ApprovalPolicy::Ask, false);
    assert!(
        !ask.approve(sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await
    );
}

/// A tool whose call has no visible detail (`tool_call_detail` is `None`) is
/// prompted with a generic sentence naming the tool — never with the SQL
/// sentence it does not match.
#[test]
fn ask_prompt_uses_the_generic_sentence_for_non_sql_tools() {
    let tools = database_tools();
    let designate = tools
        .iter()
        .find(|tool| tool.name == "designate_answer")
        .expect("designate_answer is defined");
    let prompt = approval_prompt(designate, &serde_json::json!({"sql": "SELECT 1"}));
    assert!(
        prompt.contains("Run tool `designate_answer`"),
        "a tool with no visible detail gets the generic sentence: got \"{prompt}\""
    );
    assert!(
        !prompt.contains("read-only SQL"),
        "the SQL sentence must not appear for a tool it does not describe: got \"{prompt}\""
    );
}

/// The SQL sentence is unchanged for the SQL tools — the sentence shown is the
/// one `bench/spider`-shaped usage has always been prompted with.
#[test]
fn ask_prompt_keeps_the_sql_sentence_for_sql_tools() {
    let tools = database_tools();
    let sql = tools
        .iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined");
    let prompt = approval_prompt(sql, &serde_json::json!({"sql": "SELECT 1"}));
    assert_eq!(
        prompt,
        "  SELECT 1\nAllow bounded read-only SQL query? [y/N] "
    );
}
