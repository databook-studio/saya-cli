/// Splits the event stream into maximal runs of tool events. Every
/// non-member event is a boundary: it closes the open group and is never a
/// member. No time window, no same-tool requirement — ordering defines the
/// run.
pub(crate) fn group_tool_events(events: &[AgentEvent]) -> Vec<ToolGroup> {
    let mut groups = Vec::new();
    let mut current: Vec<ToolCallPair> = Vec::new();
    let mut pending: Vec<(String, serde_json::Value, Option<ToolEffect>)> = Vec::new();

    for event in events {
        match event {
            AgentEvent::ToolRequested {
                name,
                arguments,
                effect,
            } => {
                pending.push((name.clone(), arguments.clone(), *effect));
            }
            AgentEvent::ToolCompleted { name, summary } => {
                let failed = is_failure_summary(summary);
                let nonzero_exit = !failed && is_nonzero_exit_summary(summary);
                let position = pending
                    .iter()
                    .position(|(pending_name, _, _)| pending_name == name);
                let (arguments, effect) = position
                    .map(|index| {
                        let (_, arguments, effect) = pending.remove(index);
                        (arguments, effect)
                    })
                    .unwrap_or_else(|| (serde_json::Value::Null, None));
                current.push(ToolCallPair {
                    name: name.clone(),
                    arguments,
                    effect,
                    summary: Some(summary.clone()),
                    failed,
                    nonzero_exit,
                });
            }
            _ => {
                pending.clear();
                if !current.is_empty() {
                    groups.push(ToolGroup {
                        calls: std::mem::take(&mut current),
                    });
                }
            }
        }
    }
    if !current.is_empty() {
        groups.push(ToolGroup { calls: current });
    }
    groups
}

/// Shapes one group into the lines an adapter renders: today's lines verbatim
/// for a single call, one header for an all-ok group, a header plus one
/// verbatim pair per failed call otherwise. A nonzero host exit is not a
/// `failed` summary (the "failed" substring contract is unchanged), but it is
/// still surfaced: its group carries the exit outcome in the header and
/// prints the call's verbatim pair below, so the code is never dropped from
/// the default surface.
pub(crate) fn shape_group(group: &ToolGroup) -> Vec<String> {
    if group.calls.len() <= 1 {
        return group.calls.iter().flat_map(pair_lines).collect();
    }
    let failed: Vec<&ToolCallPair> = group.calls.iter().filter(|call| call.failed).collect();
    if failed.is_empty() {
        let exited: Vec<&ToolCallPair> = group
            .calls
            .iter()
            .filter(|call| call.nonzero_exit)
            .collect();
        if exited.is_empty() {
            return vec![all_ok_header(group)];
        }
        return nonzero_exit_lines(group, &exited);
    }
    let failures = failed
        .iter()
        .map(|call| failure_label(call))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![format!(
        "▸ {} tool calls · {} failed ({}) — details below",
        group.calls.len(),
        failed.len(),
        failures
    )];
    lines.extend(failed.iter().flat_map(|call| pair_lines(call)));
    lines
}
