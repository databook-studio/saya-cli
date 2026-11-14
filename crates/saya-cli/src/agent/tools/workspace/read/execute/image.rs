//! The image arm of a workspace read.
//!
//! A file whose name says image *and* whose bytes carry that image's signature
//! is served as the image itself, beside a short true `content` — never as a
//! lossy text view of its bytes. An image the byte bound cannot carry is
//! refused whole: half an image is not the file.

use base64::Engine as _;
use saya_agent::{IMAGE_RESULT_FIELD, ToolError};
use saya_harness::workspace::ReadFile;
use serde_json::Value;

use super::super::MAX_IMAGE_BYTES;

/// Whether the path names one of the image types this read serves. The name
/// only makes the file a candidate — the bytes decide, so a text file named
/// `.png` is still read as the text it is.
pub(super) fn is_candidate(path: &str) -> bool {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("");
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp"
    )
}

/// The media type of `bytes`, from the file's own signature. The notebook read
/// sends an output as an image only when its bytes pass this same check, so
/// both paths agree on what an image is.
pub(in crate::agent::tools) fn media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    // A WebP file is a RIFF container whose form type at offset 8 is `WEBP`;
    // `RIFF` alone is also WAV and AVI, which are not images.
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// The image result: the same `size` and `digest` a text read reports — so an
/// `edit` precondition still states a state it can check — a `content` naming
/// the media type and size, and the image itself under the reserved field,
/// which the loop takes out before the result reaches the model as text.
pub(super) fn result(
    path: &str,
    media_type: &'static str,
    file: ReadFile,
) -> Result<(Value, String, u64), ToolError> {
    if file.size > MAX_IMAGE_BYTES {
        return Err(ToolError::WorkspaceImageTooLarge {
            path: path.to_owned(),
            limit: MAX_IMAGE_BYTES,
            found: file.size,
        });
    }
    let mut value = serde_json::json!({
        "path": path,
        "size": file.size,
        "digest": file.digest,
        "content": format!("{media_type}, {} bytes", file.size),
    });
    value[IMAGE_RESULT_FIELD] = serde_json::json!({
        "media_type": media_type,
        "data_base64": base64::engine::general_purpose::STANDARD.encode(&file.bytes),
    });
    Ok((value, file.digest, file.size))
}
