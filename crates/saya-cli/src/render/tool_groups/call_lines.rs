fn request_line(call: &ToolCallPair) -> String {
    let head = if call.effect.as_ref().is_some_and(read_only_permits) {
        format!("Using read-only tool: {}\n", call.name)
    } else {
        format!("Using tool: {}\n", call.name)
    };
    match key_label(&call.name, &call.arguments) {
        Some(detail) => format!("{head}  {detail}\n"),
        None => head,
    }
}

fn completion_line(call: &ToolCallPair) -> String {
    format!("{}: {}\n", call.name, call.summary.as_deref().unwrap_or(""))
}

fn pair_lines(call: &ToolCallPair) -> Vec<String> {
    vec![request_line(call), completion_line(call)]
}
