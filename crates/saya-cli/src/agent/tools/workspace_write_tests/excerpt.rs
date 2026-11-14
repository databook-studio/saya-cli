/// A new-file write carries a short numbered head of what it wrote, so the
/// model never reads the file back just to see its own content.
#[tokio::test]
async fn a_new_file_write_carries_a_numbered_head_excerpt() {
    let sandbox = Sandbox::new("write-excerpt-head");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "write",
            serde_json::json!({
                "path": "notes/new.md",
                "content": "first line\nsecond line\n",
            }),
        )
        .await
        .expect("the creating write must succeed");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("a write result carries the head excerpt");
    assert!(
        excerpt.contains("     1→first line"),
        "the first line is numbered: {excerpt}"
    );
    assert!(
        excerpt.contains("     2→second line"),
        "the second line is numbered: {excerpt}"
    );
    assert_eq!(result["digest"].as_str().map(str::len), Some(64));
}

/// The head excerpt is short: a long file shows only its first lines and
/// marks the cut.
#[tokio::test]
async fn a_write_head_excerpt_is_short_and_marks_the_cut() {
    let sandbox = Sandbox::new("write-excerpt-cut");
    let tools = sandbox.tools();
    let content = (1..=100)
        .map(|line| format!("written line {line}\n"))
        .collect::<String>();
    let result = tools
        .execute(
            "write",
            serde_json::json!({"path": "notes/long.md", "content": content}),
        )
        .await
        .expect("the creating write must succeed");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("a write result carries the head excerpt");
    let numbered = excerpt.lines().filter(|line| line.contains('→')).count();
    assert!(
        numbered <= 20,
        "at most 20 numbered head lines, got {numbered}: {excerpt}"
    );
    assert!(
        excerpt.contains("... (cut)"),
        "the cut is marked: {excerpt}"
    );
}
