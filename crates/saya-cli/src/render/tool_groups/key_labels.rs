/// The per-call key fact: `run_command` names program + argv, the SQL tools
/// reuse the `tool_call_detail` seam, the write-shaped tools name their key
/// argument. Unknown tools fall back to the bare name.
fn key_label(name: &str, arguments: &serde_json::Value) -> Option<String> {
    if name == "run_command" {
        return run_command_label(arguments);
    }
    if let Some(detail) = crate::agent::tools::tool_call_detail(name, arguments) {
        return Some(detail);
    }
    match name {
        "run_program" => string_argument(arguments, "program"),
        "http_fetch" => string_argument(arguments, "url"),
        "http_download" => string_argument(arguments, "destination"),
        "scratch_sql" => arguments
            .get("sql")
            .and_then(serde_json::Value::as_str)
            .map(|sql| sql.chars().take(24).collect::<String>())
            .filter(|sql| !sql.is_empty()),
        "scratch_import" => string_argument(arguments, "path"),
        _ => None,
    }
}

fn string_argument(arguments: &serde_json::Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn run_command_label(arguments: &serde_json::Value) -> Option<String> {
    let program = string_argument(arguments, "program")?;
    let mut parts = vec![program];
    if let Some(args) = arguments.get("args").and_then(serde_json::Value::as_array) {
        for arg in args.iter().take(2).filter_map(serde_json::Value::as_str) {
            parts.push(arg.to_owned());
        }
    }
    Some(format!("[{}]", parts.join(" ")))
}
