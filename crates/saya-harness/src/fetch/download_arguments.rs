//! The `http_download` tool lane's argument readers: the optional
//! `expected_digest` destination precondition, judged to the exact shape a
//! fresh read reports before anything is transported.
//!
//! A malformed digest refuses here — before any transport call or budget
//! use — and the refusal never names or computes a digest: the caller must
//! observe the destination's digest from a fresh read of the file, never be
//! told one, so an invented or guessed digest can never look like a usable
//! replacement.

use saya_agent::ToolError;
use serde_json::Value;

/// One optional string argument, with the digest's shape judged on arrival:
/// absent or JSON null is the create-only form (`None`); a non-string is
/// the typed argument error; anything that is not exactly 64 lowercase hex
/// characters — the shape `ReadFile::digest` reports — refuses before any
/// transport call or budget use.
pub(super) fn optional_digest_argument(
    arguments: &Value,
    name: &str,
) -> Result<Option<String>, ToolError> {
    let Some(value) = arguments
        .as_object()
        .ok_or(ToolError::ArgumentsNotObject)?
        .get(name)
    else {
        return Ok(None);
    };
    match value {
        // An explicit null is absent, the same convention the guarded file
        // tools' argument parsers follow.
        Value::Null => Ok(None),
        value => {
            let digest = value.as_str().ok_or_else(|| {
                ToolError::Fetch(format!("invalid tool arguments: {name} must be a string"))
            })?;
            if !is_lower_hex_sha256(digest) {
                return Err(ToolError::Fetch(format!(
                    "invalid tool arguments: {name} must be the 64-character lowercase hex \
                     sha256 digest you observed from a fresh read of the destination — read \
                     it first; never invent or guess a digest"
                )));
            }
            Ok(Some(digest.to_owned()))
        }
    }
}

/// Exactly 64 lowercase hex characters — the sha256 digest shape a fresh
/// read reports.
fn is_lower_hex_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
