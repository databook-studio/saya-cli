/// Parses `text` as one JSON object per line, every line, and returns the
/// objects. A blank line is tolerated (an empty stream); a non-blank line
/// that does not parse fails the test.
fn json_lines(text: &str, label: &str) -> Vec<serde_json::Value> {
    let mut objects = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("{label} line is not one JSON object: {line:?}: {error}")
        });
        assert!(
            value.is_object(),
            "{label} line must be one JSON object: {line:?}"
        );
        objects.push(value);
    }
    objects
}

/// Collects the `"type"` tags of the streamed `RunEvent` lines.
fn type_tags(values: &[serde_json::Value]) -> Vec<&str> {
    values
        .iter()
        .filter_map(|value| value.get("type").and_then(|tag| tag.as_str()))
        .collect()
}
