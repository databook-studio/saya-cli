//! A ranged read returns one numbered rendering of the selected lines —
//! `   123→text`, the form the model reads a file in. There is no second raw
//! copy, so what one call can carry is the numbered size rather than twice it,
//! and the whole-file digest the edit path anchors on is untouched.

use super::Sandbox;
use saya_agent::ToolExecutor;

/// The window fixture: 4,999 lines of 54 bytes and one 40,006-byte tail line,
/// so `total_lines` is 5,000 and a selection at line 4,000 holds 1,001 lines.
/// The 54-byte lines put the thousandth newline just inside the model-facing
/// byte cap, which then stops inside the long tail line: the served prefix is
/// exactly 1,000 complete lines and `selected_range_complete` is false.
const WINDOW_LINES: usize = 5_000;
const WINDOW_START_LINE: usize = 4_000;
const WINDOW_SHORT_LINE_BYTES: usize = 54;

fn window_short_line(number: usize) -> String {
    format!("{number:04} {}\n", "x".repeat(WINDOW_SHORT_LINE_BYTES - 6))
}

fn window_text() -> String {
    let mut text = String::new();
    for number in 1..WINDOW_LINES {
        text.push_str(&window_short_line(number));
    }
    text.push_str(&format!("tail {}\n", "y".repeat(40_000)));
    text
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// An un-ranged read is every line from line 1, numbered exactly as a ranged
/// read numbers its selection.
#[tokio::test]
async fn an_unranged_read_numbers_every_line() {
    let sandbox = Sandbox::new("unranged-numbered");
    sandbox
        .ws
        .write("three.txt", b"a\nb\nc")
        .expect("seed write must succeed");
    let result = sandbox
        .tools()
        .execute("read", serde_json::json!({"path": "three.txt"}))
        .await
        .expect("a contained read must succeed");
    assert_eq!(result["content"], "     1→a\n     2→b\n     3→c");
}

/// The prefix is a rendering, never file bytes: an anchor copied from the
/// un-numbered file content still matches, and no prefix reaches the file.
#[tokio::test]
async fn an_edit_anchor_from_the_un_numbered_file_still_matches() {
    let sandbox = Sandbox::new("unranged-anchor");
    sandbox
        .ws
        .write("three.txt", b"a\nb\nc\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let read = tools
        .execute("read", serde_json::json!({"path": "three.txt"}))
        .await
        .expect("a contained read must succeed");
    assert_eq!(read["content"], "     1→a\n     2→b\n     3→c\n");

    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "three.txt",
                "old_text": "a\nb\n",
                "new_text": "a\nB\n",
            }),
        )
        .await
        .expect("the un-numbered file text is the anchor");
    assert_eq!(
        sandbox
            .ws
            .read("three.txt", 1_024)
            .expect("edited file")
            .bytes,
        b"a\nB\nc\n",
        "the edit wrote the anchor, never a number prefix"
    );
}

/// An un-ranged read is still bounded by the same 2,000-line window as a
/// ranged one, and says so with `truncated`.
#[tokio::test]
async fn an_unranged_read_stops_at_the_two_thousand_line_window() {
    let sandbox = Sandbox::new("unranged-window");
    sandbox
        .ws
        .write("long.txt", "x\n".repeat(2_500).as_bytes())
        .expect("seed write must succeed");
    let result = sandbox
        .tools()
        .execute("read", serde_json::json!({"path": "long.txt"}))
        .await
        .expect("a contained read must succeed");
    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert_eq!(content.matches('\n').count(), 2_000);
    assert!(content.starts_with("     1→x\n"));
    assert!(
        content.ends_with("  2000→x\n"),
        "the window ends at line 2,000"
    );
    assert_eq!(result["truncated"], true);
}

#[tokio::test]
async fn a_two_thousand_line_window_serves_a_bounded_prefix_and_says_so() {
    let sandbox = Sandbox::new("numbered-window");
    let text = window_text();
    sandbox
        .ws
        .write("big.txt", text.as_bytes())
        .expect("seed write must succeed");
    let result = sandbox
        .tools()
        .execute(
            "read",
            serde_json::json!({
                "path": "big.txt",
                "start_line": WINDOW_START_LINE,
                "line_count": 2000
            }),
        )
        .await
        .expect("a 2000-line window must be accepted");

    assert_eq!(result["line_count"], 2000);
    assert_eq!(result["start_line"], WINDOW_START_LINE);
    assert_eq!(
        result["total_lines"].as_u64(),
        Some(WINDOW_LINES as u64),
        "the count of lines in the file is reported truthfully"
    );
    assert_eq!(result["size"].as_u64(), Some(text.len() as u64));
    assert_eq!(result["selected_range_complete"], false);
    assert_eq!(result["truncated"], true);
    assert_eq!(
        result["lines_returned"].as_u64(),
        Some(1_000),
        "the 1001-line selection does not fit the byte cap; the served prefix is 1000 lines"
    );
    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert_eq!(content.matches('\n').count(), 1_000);
    assert!(
        content.starts_with(&format!(
            "{:>6}→{}",
            WINDOW_START_LINE,
            window_short_line(4_000)
        )),
        "the served text starts at the requested line, numbered"
    );
    assert_eq!(result["digest"], sha256_hex(text.as_bytes()));
}

#[tokio::test]
async fn the_numbered_lines_match_the_start_line_offsets() {
    let sandbox = Sandbox::new("numbered-offsets");
    sandbox
        .ws
        .write("small.txt", b"alpha\nbeta\ngamma\ndelta\nepsilon\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();

    let ranged = tools
        .execute(
            "read",
            serde_json::json!({"path": "small.txt", "start_line": 3, "line_count": 2}),
        )
        .await
        .expect("a valid line range must be readable");
    assert_eq!(ranged["content"], "     3→gamma\n     4→delta\n");

    let defaulted = tools
        .execute(
            "read",
            serde_json::json!({"path": "small.txt", "line_count": 2}),
        )
        .await
        .expect("a range without start_line starts at line 1");
    assert_eq!(defaulted["start_line"], 1);
    assert_eq!(defaulted["content"], "     1→alpha\n     2→beta\n");

    let past_end = tools
        .execute(
            "read",
            serde_json::json!({"path": "small.txt", "start_line": 9, "line_count": 2}),
        )
        .await
        .expect("a window past the end selects nothing");
    assert_eq!(past_end["content"], "");
}

/// The numbered text is the only copy: a 500-line window that fits is served
/// once, at the numbered size — not the raw bytes plus a numbered rendering,
/// which is what doubled the tokens and halved the window that fit.
#[tokio::test]
async fn a_ranged_read_carries_the_numbered_text_once() {
    let sandbox = Sandbox::new("numbered-once");
    let text = (1..=500)
        .map(|line| format!("{line:04} {}\n", "x".repeat(59)))
        .collect::<String>();
    sandbox
        .ws
        .write("five-hundred.txt", text.as_bytes())
        .expect("seed write must succeed");
    let result = sandbox
        .tools()
        .execute(
            "read",
            serde_json::json!({"path": "five-hundred.txt", "line_count": 2000}),
        )
        .await
        .expect("a 500-line window fits and must be served whole");

    assert!(
        result.get("lines").is_none(),
        "there is no second rendering beside content"
    );
    assert_eq!(result["selected_range_complete"], true);
    assert_eq!(result["lines_returned"], 500);
    let expected = (1..=500)
        .map(|line| format!("{line:>6}→{line:04} {}\n", "x".repeat(59)))
        .collect::<String>();
    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert_eq!(
        content, expected,
        "the served text is the numbered form of the file's own bytes"
    );

    let served = serde_json::to_string(&result)
        .expect("the result serializes")
        .len();
    assert!(
        served < content.len() + 1_024,
        "one rendering plus the range metadata: {served} bytes for {} numbered bytes",
        content.len()
    );
    assert!(
        served < 2 * text.len(),
        "the raw text is not sent as well: {served} bytes for a {}-byte file",
        text.len()
    );
    assert_eq!(result["digest"], sha256_hex(text.as_bytes()));
}
