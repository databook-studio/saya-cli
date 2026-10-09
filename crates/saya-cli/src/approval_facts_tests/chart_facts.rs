use super::*;

#[test]
fn render_chart_save_prompt_names_path_and_replacement_risk() {
    let tool = database_tool("render_chart");
    let arguments = serde_json::json!({
        "sql": "SELECT region, count(*) FROM orders GROUP BY 1",
        "connection": "analytics",
        "chart_type": "bar",
        "save_to": "reports/\nchart.html"
    });
    let prompt = approval_prompt(
        &tool,
        &arguments,
        None,
        &sql_facts(),
        Some("analytics"),
        None,
    );
    assert!(prompt.contains("save_to: reports/ chart.html"));
    assert!(prompt.contains("creates the file only; an existing file is refused"));
}

/// With a digest the call states the guarded replace: the existing file is
/// replaced only if its content matches the digest the caller observed —
/// and the create-only line is absent.
#[test]
fn render_chart_save_prompt_states_replacement_only_with_a_digest() {
    let tool = database_tool("render_chart");
    let arguments = serde_json::json!({
        "sql": "SELECT region, count(*) FROM orders GROUP BY 1",
        "chart_type": "bar",
        "save_to": "reports/chart.html",
        "expected_digest": "a".repeat(64)
    });
    let prompt = approval_prompt(&tool, &arguments, None, &sql_facts(), None, None);
    assert!(
        prompt.contains(
            "replaces the existing file only if its content matches the digest you observed"
        ),
        "the replacement line with a digest: {prompt}"
    );
    assert!(
        !prompt.contains("creates the file only"),
        "no create-only line next to a digest: {prompt}"
    );
}

/// Without a digest the prompt states the create-only form: the file is
/// created only, and an existing file is refused.
#[test]
fn render_chart_save_prompt_states_create_only_without_a_digest() {
    let tool = database_tool("render_chart");
    let arguments = serde_json::json!({
        "sql": "SELECT region, count(*) FROM orders GROUP BY 1",
        "chart_type": "bar",
        "save_to": "reports/chart.html"
    });
    let prompt = approval_prompt(&tool, &arguments, None, &sql_facts(), None, None);
    assert!(
        prompt.contains("creates the file only; an existing file is refused"),
        "the create-only line without a digest: {prompt}"
    );
    assert!(
        !prompt.contains("replaces the existing file"),
        "no replacement line without a digest: {prompt}"
    );
}

/// A present-but-unusable digest (a non-string the call refuses at
/// validation) states neither destination semantics — a prompt may not claim
/// a bound the code does not apply.
#[test]
fn render_chart_save_prompt_states_no_destination_semantics_for_a_malformed_digest() {
    let tool = database_tool("render_chart");
    let arguments = serde_json::json!({
        "sql": "SELECT region, count(*) FROM orders GROUP BY 1",
        "chart_type": "bar",
        "save_to": "reports/chart.html",
        "expected_digest": 7
    });
    let prompt = approval_prompt(&tool, &arguments, None, &sql_facts(), None, None);
    assert!(
        !prompt.contains("creates the file only"),
        "no create-only claim for a call that refuses: {prompt}"
    );
    assert!(
        !prompt.contains("replaces the existing file"),
        "no replacement claim for a call that refuses: {prompt}"
    );
}
