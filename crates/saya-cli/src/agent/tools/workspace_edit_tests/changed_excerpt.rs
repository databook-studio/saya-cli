/// An edit result carries the changed lines, numbered as the read tool
/// numbers them, with a few lines of context — so the model sees what landed
/// without spending a read turn.
#[tokio::test]
async fn an_edit_result_carries_the_changed_lines_with_their_numbers() {
    let sandbox = Sandbox::new("excerpt-lines");
    sandbox
        .ws
        .write(
            "notes.md",
            b"line one\nline two\nline three\nline four\nline five\nline six\nline seven\nline eight\nline nine\nline ten\n",
        )
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "line five",
                "new_text": "LINE FIVE",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("the edit must apply");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("an edit result carries the changed excerpt");
    assert!(
        excerpt.contains("     5→LINE FIVE"),
        "the changed line is numbered: {excerpt}"
    );
    assert!(
        excerpt.contains("     4→line four"),
        "context before the change is included: {excerpt}"
    );
    assert!(
        excerpt.contains("     6→line six"),
        "context after the change is included: {excerpt}"
    );
    assert!(
        !excerpt.contains("line one") && !excerpt.contains("line ten"),
        "the excerpt is the change, not the whole file: {excerpt}"
    );
    assert_eq!(result["digest"].as_str().map(str::len), Some(64));
}

/// The excerpt is capped: a replacement spanning more than 40 lines shows at
/// most 40 numbered lines and marks the cut.
#[tokio::test]
async fn an_edit_excerpt_stops_at_forty_lines_and_marks_the_cut() {
    let sandbox = Sandbox::new("excerpt-line-cap");
    sandbox
        .ws
        .write("notes.md", b"start\n")
        .expect("seed write must succeed");
    let replacement = (1..=200)
        .map(|line| format!("new line {line}\n"))
        .collect::<String>();
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "start",
                "new_text": replacement,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("the edit must apply");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("an edit result carries the changed excerpt");
    let numbered = excerpt.lines().filter(|line| line.contains('→')).count();
    assert!(
        numbered <= 40,
        "at most 40 numbered lines per hunk, got {numbered}: {excerpt}"
    );
    assert!(
        excerpt.contains("... (cut)"),
        "the cut is marked: {excerpt}"
    );
}

/// A single changed line longer than the hunk budget is cut at a character
/// boundary, and the hunk stays within 4 KiB.
#[tokio::test]
async fn an_edit_excerpt_cuts_a_long_line_at_the_hunk_budget() {
    let sandbox = Sandbox::new("excerpt-byte-cap");
    sandbox
        .ws
        .write("notes.md", b"start\n")
        .expect("seed write must succeed");
    let replacement = "x".repeat(8 * 1024);
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "start",
                "new_text": replacement,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("the edit must apply");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("an edit result carries the changed excerpt");
    assert!(
        excerpt.len() <= 4 * 1024,
        "one hunk stays within 4 KiB, got {} bytes",
        excerpt.len()
    );
    assert!(
        excerpt.contains("... (cut)"),
        "the cut is marked: {}",
        &excerpt[..excerpt.len().min(80)]
    );
}

/// Many separate hunks stop at the 12 KiB per-result budget, not at the
/// number of hunks.
#[tokio::test]
async fn a_multi_edit_excerpt_stops_at_the_result_budget() {
    let sandbox = Sandbox::new("excerpt-result-cap");
    let mut seed = String::new();
    let mut edits = Vec::new();
    for anchor in 0..32 {
        seed.push_str(&format!("anchor-{anchor:02}\n"));
        for filler in 0..10 {
            seed.push_str(&format!("filler-{anchor:02}-{filler:02}\n"));
        }
        edits.push(serde_json::json!({
            "old_text": format!("anchor-{anchor:02}"),
            "new_text": format!("new-{anchor:02}-{}", "x".repeat(500)),
        }));
    }
    sandbox
        .ws
        .write("notes.md", seed.as_bytes())
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
                "edits": edits,
            }),
        )
        .await
        .expect("the multi edit must apply");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("an edit result carries the changed excerpt");
    assert!(
        excerpt.len() <= 12 * 1024,
        "the whole excerpt stays within 12 KiB, got {} bytes",
        excerpt.len()
    );
    assert!(
        excerpt.contains("... (cut)"),
        "the result-level cut is marked: {}",
        &excerpt[..excerpt.len().min(80)]
    );
}

/// A replace_all result reports every changed site it showed, each numbered.
#[tokio::test]
async fn a_replace_all_excerpt_numbers_every_changed_site() {
    let sandbox = Sandbox::new("excerpt-replace-all");
    sandbox
        .ws
        .write("notes.md", b"needle one\nfiller\nneedle two\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "needle",
                "new_text": "thread",
                "replace_all": true,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("replace_all must apply");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("an edit result carries the changed excerpt");
    assert!(
        excerpt.contains("     1→thread one"),
        "the first site is numbered: {excerpt}"
    );
    assert!(
        excerpt.contains("     3→thread two"),
        "the second site is numbered: {excerpt}"
    );
}

/// An append reports the appended lines at their resulting line numbers.
#[tokio::test]
async fn an_append_excerpt_numbers_the_appended_lines() {
    let sandbox = Sandbox::new("excerpt-append");
    sandbox
        .ws
        .write("notes.md", b"first\nsecond\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "offset": 13,
                "chunk": "third\n",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("the append must apply");
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("an edit result carries the changed excerpt");
    assert!(
        excerpt.contains("     3→third"),
        "the appended line carries its resulting number: {excerpt}"
    );
}
