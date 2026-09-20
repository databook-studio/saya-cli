use super::super::{BlockKind, Transcript};

fn flushed(t: &Transcript) -> String {
    t.blocks()
        .iter()
        .flat_map(|b| {
            let mut v = vec![b.text.clone()];
            if let Some(g) = &b.group {
                v.extend(g.detail.clone());
            }
            v
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Two parallel calls of one tool must keep their own results. The loop
/// emits all requests then all completions, both in `tool_calls` order, so
/// pairing is FIFO; matching newest-first showed one call's arguments
/// beside another call's result once the group was expanded.
#[test]
fn a_flushed_group_keeps_each_summary_with_its_own_arguments() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "compare the two regions");
    t.buffer_tool_request("sql".into(), serde_json::json!({"q": "north"}), None);
    t.buffer_tool_request("sql".into(), serde_json::json!({"q": "south"}), None);
    t.buffer_tool_completion("sql", "north: 12 rows");
    t.buffer_tool_completion("sql", "south: 400 rows");
    t.flush_tool_buffer(
        |name, args| vec![format!("→ {name} {args}")],
        |name, summary| format!("✓ {name}: {summary}"),
    );
    let text = flushed(&t);
    let north_q = text.find(r#"→ sql {"q":"north"}"#).expect("north request");
    let north_r = text.find("✓ sql: north: 12 rows").expect("north result");
    let south_q = text.find(r#"→ sql {"q":"south"}"#).expect("south request");
    let south_r = text.find("✓ sql: south: 400 rows").expect("south result");
    assert!(
        north_q < north_r && north_r < south_q && south_q < south_r,
        "each result must follow its own request:\n{text}"
    );
}

/// The sequential path is unaffected: only one call of a name is ever open
/// at a time, so FIFO and newest-first agree.
#[test]
fn sequential_calls_of_one_tool_are_unchanged() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "two questions");
    t.buffer_tool_request("sql".into(), serde_json::json!({"q": "first"}), None);
    t.buffer_tool_completion("sql", "first: ok");
    t.buffer_tool_request("sql".into(), serde_json::json!({"q": "second"}), None);
    t.buffer_tool_completion("sql", "second: ok");
    t.flush_tool_buffer(
        |name, args| vec![format!("→ {name} {args}")],
        |name, summary| format!("✓ {name}: {summary}"),
    );
    let text = flushed(&t);
    assert!(
        text.find("first: ok").unwrap() < text.find(r#"{"q":"second"}"#).unwrap(),
        "sequential order preserved:\n{text}"
    );
}
