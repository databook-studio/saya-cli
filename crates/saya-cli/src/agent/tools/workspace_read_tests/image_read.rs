//! Tests for the media arm of `read`: a supported image is returned as an
//! image block beside a short, true text content — never as a lossy text view
//! of its bytes — and an image the bound cannot carry is refused whole.

use super::*;
use base64::Engine as _;
use saya_agent::IMAGE_RESULT_FIELD;

/// A real 1×1 PNG: the bytes a provider is asked to decode, not a header the
/// read merely recognises.
const PNG_1X1_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn png_bytes() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(PNG_1X1_BASE64)
        .expect("the PNG fixture must decode")
}

#[tokio::test]
async fn read_of_a_png_returns_the_image_beside_a_short_true_content() {
    let sandbox = Sandbox::new("image-png");
    let bytes = png_bytes();
    sandbox
        .ws
        .write("assets/pixel.png", &bytes)
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    let result = tools
        .execute("read", serde_json::json!({"path": "assets/pixel.png"}))
        .await
        .expect("a contained image read must succeed");

    assert_eq!(result["path"], "assets/pixel.png");
    assert_eq!(result["size"].as_u64(), Some(bytes.len() as u64));
    assert_eq!(
        result["content"],
        format!("image/png, {} bytes", bytes.len())
    );
    assert!(
        result.get("truncated").is_none(),
        "a whole-image read carries no `truncated`: {result}"
    );
    let image = &result[IMAGE_RESULT_FIELD];
    assert_eq!(image["media_type"], "image/png");
    assert_eq!(
        image["data_base64"], PNG_1X1_BASE64,
        "the image rides base64 of the file's own bytes"
    );
    assert!(
        result["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "an image read still names the file state an edit precondition states: {result}"
    );
}

#[tokio::test]
async fn read_of_a_gif_and_a_webp_returns_an_image() {
    let sandbox = Sandbox::new("image-others");
    sandbox
        .ws
        .write("a.gif", b"GIF89a\x01\x00\x01\x00\x00\x00\x00;")
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("b.webp", b"RIFF\x1a\x00\x00\x00WEBPVP8 \x0e\x00\x00\x00")
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    for (path, media_type) in [("a.gif", "image/gif"), ("b.webp", "image/webp")] {
        let result = tools
            .execute("read", serde_json::json!({ "path": path }))
            .await
            .unwrap_or_else(|error| panic!("{path} must read as an image: {error}"));
        assert_eq!(result[IMAGE_RESULT_FIELD]["media_type"], media_type);
        assert_eq!(
            result["content"],
            format!("{media_type}, {} bytes", result["size"].as_u64().unwrap())
        );
    }
}

/// An image over the bound is refused whole, and the refusal names the bound
/// and the file's real size — never a truncated image the model would read as
/// the file, and never a text view of its bytes.
#[tokio::test]
async fn an_image_over_the_byte_bound_is_refused_whole() {
    let sandbox = Sandbox::new("image-too-large");
    let mut bytes = png_bytes();
    bytes.resize(MAX_IMAGE_BYTES as usize + 1, 0);
    sandbox
        .ws
        .write("huge.png", &bytes)
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    let error = tools
        .execute("read", serde_json::json!({"path": "huge.png"}))
        .await
        .expect_err("an over-bound image must be refused");
    match error {
        ToolError::WorkspaceImageTooLarge { found, limit, .. } => {
            assert_eq!(found, bytes.len() as u64);
            assert_eq!(limit, MAX_IMAGE_BYTES);
        }
        other => panic!("expected the image bound refusal, got {other}"),
    }
}

/// The bound is on the file, not on the extension: a file whose extension
/// says image but whose bytes do not carry that image's signature is read as
/// the text it is, so a mislabelled file never becomes a broken image block.
#[tokio::test]
async fn a_file_named_like_an_image_but_not_one_is_read_as_text() {
    let sandbox = Sandbox::new("image-mislabelled");
    sandbox
        .ws
        .write("notes.png", b"just notes")
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    let result = tools
        .execute("read", serde_json::json!({"path": "notes.png"}))
        .await
        .expect("a mislabelled file is still readable");
    assert_eq!(result["content"], "     1→just notes");
    assert!(
        result.get(IMAGE_RESULT_FIELD).is_none(),
        "no image block for a file that is not an image: {result}"
    );
}

/// An image has no lines: a line range on one is refused with a typed reason
/// rather than answered from a lossy text view of the image's bytes.
#[tokio::test]
async fn a_line_range_on_an_image_is_refused() {
    let sandbox = Sandbox::new("image-range");
    sandbox
        .ws
        .write("pixel.png", &png_bytes())
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    let error = tools
        .execute(
            "read",
            serde_json::json!({"path": "pixel.png", "start_line": 1, "line_count": 10}),
        )
        .await
        .expect_err("a line range on an image must be refused");
    assert!(
        matches!(error, ToolError::WorkspaceImageLineRange { .. }),
        "expected the line-range refusal, got {error}"
    );
}
