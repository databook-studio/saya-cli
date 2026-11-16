/// Runs the real `saya ask` headlessly with the mock as provider and the
/// given budget variables planted on the child only. Every other budget
/// variable is scrubbed from the child so no scenario inherits an ambient
/// value. Copied from `tests/correctness/common.rs`, plus `budgets`.
fn run_ask(
    fixture: &DemoFixture,
    mock_address: &str,
    prompt: &str,
    budgets: &[(&str, &str)],
) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args([
            "--non-interactive",
            "--format",
            "ndjson",
            "--connections",
            fixture.connections.to_str().unwrap(),
            "--profile",
            "demo",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "ask",
            prompt,
        ])
        .env("SAYA_CONFIG_HOME", &fixture.config_home)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{mock_address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &fixture.state_db)
        .env("SAYA_ALLOW_DATA_SHARING", "true")
        .env("HOME", &fixture.root);
    for name in BUDGET_VARS {
        command.env_remove(name);
    }
    for (name, value) in budgets {
        command.env(name, value);
    }
    command.output().unwrap()
}

/// The JSON body of a captured raw HTTP request (headers + body).
fn request_json(raw: &str) -> serde_json::Value {
    let body = raw.rsplit("\r\n\r\n").next().unwrap_or(raw);
    serde_json::from_str(body).unwrap()
}

/// Whether the captured request advertised tool definitions — the loop's
/// answering turns do; the ceiling salvage call strips them.
fn request_has_tools(raw: &str) -> bool {
    request_json(raw)
        .get("tools")
        .and_then(|tools| tools.as_array())
        .is_some_and(|tools| !tools.is_empty())
}

/// Twelve scripted `bounded_sql_query` rounds against the demo fixture: each
/// returns one tool call on a real table, and the run's own work (plus the
/// always-available text fallback) always terminates it — so a dropped
/// ceiling degrades to a completed run instead of hanging the test.
fn tool_call_script(rounds: usize) -> Vec<String> {
    (0..rounds)
        .map(|round| {
            tool_call_body(
                &format!("call-{round}"),
                "bounded_sql_query",
                serde_json::json!({
                    "sql": "SELECT COUNT(*) FROM orders",
                }),
            )
        })
        .collect()
}

/// Three scripted `bounded_sql_query` rounds followed by a plain-text answer.
/// The text sits where the ceiling salvage call lands: under the planted
/// ceiling the loop stops after three answering turns and salvages this
/// text; a dropped ceiling degrades to a natural completion on the same
/// text instead of hanging. Either way the run terminates with an answer —
/// the ceiling stop is distinguished by the requests' tools signature (the
/// salvage call strips the tool definitions), not by the answer.
fn three_rounds_then_text(answer: &str) -> Vec<String> {
    let mut script = tool_call_script(3);
    script.push(text_body(answer));
    script
}

/// Runs one scenario and returns what the mock received plus the child's
/// output: `(request_count, tools_on_each_request, child_output)`.
fn run_scenario(
    label: &str,
    script: Vec<String>,
    budgets: &[(&str, &str)],
) -> (usize, Vec<bool>, std::process::Output) {
    let fixture = build_demo(label);
    let mut mock = spawn_mock(script);
    let output = run_ask(&fixture, mock.address(), "how many orders exist", budgets);
    mock.join();
    let bodies = mock.bodies();
    let tools = bodies.iter().map(|body| request_has_tools(body)).collect();
    let _ = fs::remove_dir_all(&fixture.root);
    (bodies.len(), tools, output)
}
