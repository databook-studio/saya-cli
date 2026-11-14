fn all_ok_header(group: &ToolGroup) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut keyed: HashMap<&str, Vec<String>> = HashMap::new();
    for call in &group.calls {
        if !counts.contains_key(call.name.as_str()) {
            order.push(call.name.clone());
            counts.insert(call.name.as_str(), 0);
            keyed.insert(call.name.as_str(), Vec::new());
        }
        if let Some(count) = counts.get_mut(call.name.as_str()) {
            *count += 1;
        }
        if let Some(label) = key_label(&call.name, &call.arguments)
            && let Some(entry) = keyed.get_mut(call.name.as_str())
        {
            entry.push(label);
        }
    }
    let segments = order
        .iter()
        .map(|name| ok_segment(name, &counts, &keyed))
        .collect::<Vec<_>>()
        .join(", ");
    format!("▸ {} tool calls · ok — {segments}", group.calls.len())
}

fn ok_segment(
    name: &str,
    counts: &HashMap<&str, usize>,
    keyed: &HashMap<&str, Vec<String>>,
) -> String {
    let empty = Vec::new();
    let entry = keyed.get(name).unwrap_or(&empty);
    if entry.is_empty() {
        if counts.get(name).copied().unwrap_or(1) > 1 {
            return format!("{name} ×{}", counts[name]);
        }
        return name.to_owned();
    }
    let shown: Vec<&str> = entry.iter().take(3).map(String::as_str).collect();
    let mut segment = format!("{name} {}", shown.join(", "));
    if entry.len() > 3 {
        segment.push_str(&format!(" +{} more", entry.len() - 3));
    }
    segment
}

/// The nonzero-exit group shape: a header carrying the outcome (program and
/// exit code per call) plus one verbatim pair per exited call — the same
/// never-collapse posture as the failure shape, without reclassifying the
/// summary as "failed".
fn nonzero_exit_lines(group: &ToolGroup, exited: &[&ToolCallPair]) -> Vec<String> {
    let outcomes = exited
        .iter()
        .map(|call| call.summary.as_deref().unwrap_or_default().to_owned())
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![format!(
        "▸ {} tool calls · exited nonzero ({}) — details below",
        group.calls.len(),
        outcomes
    )];
    lines.extend(exited.iter().flat_map(|call| pair_lines(call)));
    lines
}

fn failure_label(call: &ToolCallPair) -> String {
    match key_label(&call.name, &call.arguments) {
        Some(label) if call.name == "run_command" => format!("{} {label}", call.name),
        Some(label) => format!("{} [{label}]", call.name),
        None => call.name.clone(),
    }
}
