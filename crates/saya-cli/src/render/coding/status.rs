use super::text_of;
use serde_json::Value;
pub(super) fn show(value: &Value) -> Option<String> {
    let job = value.get("job_id")?.as_str()?;
    let children = value.get("children")?.as_array()?;
    let count = match children.len() {
        0 => "no child agents".to_owned(),
        1 => "1 child agent".to_owned(),
        many => format!("{many} child agents"),
    };
    let mut lines = vec![format!(
        "Job {job}: {}, {count}",
        text_of(value.get("state")?)
    )];
    lines.extend(stop_line(value));
    lines.extend(usage_line(value));
    lines.extend(progress_line(value));
    for child in children {
        let mut line = format!(
            "  {}  {}",
            child.get("agent_id")?.as_str()?,
            text_of(child.get("state")?)
        );
        let names = |key: &str| -> Vec<&str> {
            child
                .get(key)
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default()
        };
        let paths = names("write_paths");
        if !paths.is_empty() {
            line.push_str(&format!("  owns {}", paths.join(", ")));
        }
        let waiting = names("blocked_by");
        if !waiting.is_empty() {
            line.push_str(&format!("  waiting on {}", waiting.join(", ")));
        }
        if let Some(invocation) = child.get("invocation_id").and_then(Value::as_str) {
            line.push_str(&format!("  invocation {invocation}"));
        }
        if let Some(count) = child["invocation_lineage"]["follow_ups_prepared"].as_u64() {
            line.push_str(&format!("  follow-ups {count}/3"));
        }
        lines.push(line);
    }
    if value
        .get("next_cursor")
        .is_some_and(|cursor| !cursor.is_null())
    {
        lines.push("  … more child agents exist".into());
    }
    Some(lines.join("\n"))
}

/// Provider-reported usage, shown only when the job recorded any request.
fn usage_line(value: &Value) -> Option<String> {
    let usage = value.get("usage")?;
    let requests = usage.get("requests")?.as_u64()?;
    if requests == 0 {
        return None;
    }
    let noun = if requests == 1 { "request" } else { "requests" };
    let mut line = format!(
        "  Usage: {requests} {noun}, {} input",
        usage.get("input")?.as_u64()?
    );
    if let Some(cached) = usage.get("cached_input").and_then(Value::as_u64) {
        line.push_str(&format!(" ({cached} cached)"));
    }
    line.push_str(&format!(", {} output", usage.get("output")?.as_u64()?));
    if let Some(reasoning) = usage.get("reasoning").and_then(Value::as_u64) {
        line.push_str(&format!(", {reasoning} reasoning"));
    }
    if let Some(wall_time) = usage.get("model_wall_time_ms").and_then(Value::as_u64) {
        line.push_str(&format!(", {wall_time} ms model"));
    }
    if let Some(cost) = usage.get("cost_usd").and_then(Value::as_f64) {
        line.push_str(&format!(", ${cost:.4}"));
    }
    if usage.get("complete").and_then(Value::as_bool) == Some(false) {
        line.push_str(" (incomplete)");
    }
    Some(line)
}

/// The loop's own progress report, shown only when the job recorded one.
fn progress_line(value: &Value) -> Option<String> {
    let progress = value.get("progress")?;
    let turns = progress.get("turns")?.as_u64()?;
    if turns == 0 {
        return None;
    }
    let events: u64 = progress
        .get("progress_events")
        .and_then(Value::as_object)?
        .values()
        .filter_map(Value::as_u64)
        .sum();
    let last = progress
        .get("last_progress_turn")
        .and_then(Value::as_u64)
        .map(|turn| format!(", last at turn {turn}"))
        .unwrap_or_default();
    let check = progress
        .get("check")
        .and_then(|check| check.get("message"))
        .and_then(Value::as_str)
        .map(|message| format!(" — {message}"))
        .unwrap_or_default();
    Some(format!(
        "  Progress: {turns} turns, {events} events{last}{check}"
    ))
}

/// The closed class a failed job stopped with, and the child it came from when it had one.
fn stop_line(value: &Value) -> Option<String> {
    let stop = value.get("failure")?.get("event")?.get("failure")?;
    let class = stop.get("class")?.as_str()?;
    Some(match stop.get("agent_id").and_then(Value::as_str) {
        Some(agent) => format!("  Stopped: {class} (agent {agent})"),
        None => format!("  Stopped: {class}"),
    })
}
