/// Every regular file under `dir`, recursively (for the sentinel scans).
fn files_under(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Every stdout ndjson event, parsed.
fn events(out: &str) -> Vec<serde_json::Value> {
    out.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|error| panic!("stdout line is not JSON: {line:?}: {error}"))
        })
        .collect()
}

/// The tool names a captured request advertises, owned.
fn names_of(request: &serde_json::Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .expect("request advertises tools")
        .iter()
        .map(|tool| {
            tool["function"]["name"]
                .as_str()
                .expect("tool names its function")
                .to_owned()
        })
        .collect()
}

/// The `tool_completed` events' (name, summary) pairs in order.
fn completions(out: &str) -> Vec<(String, String)> {
    events(out)
        .into_iter()
        .filter_map(|event| {
            if event["event"] != "tool_completed" {
                return None;
            }
            Some((
                event["name"].as_str()?.to_owned(),
                event["summary"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// The one terminal result's status.
fn turn_result_status(out: &str) -> String {
    events(out)
        .into_iter()
        .find(|event| event["event"] == "turn_result")
        .expect("the stream ends with one turn_result")["status"]
        .as_str()
        .expect("turn_result carries a status")
        .to_owned()
}

/// The model-facing tool result the LAST captured request carries.
fn last_tool_result(raw: &str) -> String {
    tool_results(&request_json(raw))
        .last()
        .copied()
        .expect("the request carries a tool result")
        .to_owned()
}

/// Every regular file under `dir`, recursively, or the file itself.
fn scan_targets(path: &std::path::Path) -> Vec<PathBuf> {
    if path.is_dir() {
        files_under(path)
    } else {
        vec![path.to_path_buf()]
    }
}

/// No persisted file under the session dir or the state database carries the
/// secret sentinel (bytes → lossy text scan).
fn assert_secret_stays_out_of_persistence(env: &Env) {
    for dir in [&env.sessions, &env.root.join("state.sqlite3")] {
        for file in scan_targets(dir) {
            let bytes = std::fs::read(&file).unwrap_or_default();
            assert!(
                !String::from_utf8_lossy(&bytes).contains(SECRET),
                "secret sentinel leaked into {}",
                file.display()
            );
        }
    }
}
