//! The shape a budget-trimmed tool result takes. A tool result is JSON, so
//! cutting it at a byte offset and appending the marker hands the model a
//! string cut mid-way — invalid JSON the provider rejects. JSON content is
//! therefore rewritten as a valid-JSON envelope carrying the surviving prefix;
//! non-JSON content keeps the prefix-plus-marker form. A sibling of
//! `context_budget` so that file stays inside its size limit.

use super::context_budget::TRUNCATED_MARKER;
use std::borrow::Cow;

/// The note carried by a truncated JSON tool result, beside the original size.
const NOTE: &str = "tool result exceeded the conversation byte budget";
/// The smallest envelope, for a target with no room for the original size
/// beside the flag. Every tier is valid JSON, so this one is too.
const MINIMAL_ENVELOPE: &str = "{\"truncated\":true}";
/// The envelope's opening bytes, shared by every tier. A cheap shape check
/// keeps a result the trim has not touched from paying for a parse it does not
/// need.
const ENVELOPE_PREFIX: &str = "{\"truncated\":true";

/// The longest form of `content` that fits `allowed` bytes, or `None` when
/// nothing can be shortened and the caller must leave the message alone.
pub(super) fn truncate(content: &str, allowed: usize) -> Option<String> {
    let (text, original_bytes) = match prior_envelope(content) {
        Some(prior) => (Cow::Owned(prior.partial), prior.original_bytes),
        // One allocation-free validation pass decides JSON from non-JSON; a
        // tool result is JSON, so the answer is JSON on nearly every call.
        None if serde_json::from_str::<serde::de::IgnoredAny>(content).is_ok() => {
            (Cow::Borrowed(content), content.len())
        }
        None => return marker_prefix(content, allowed),
    };
    let replacement = envelope(&text, original_bytes, allowed);
    (replacement.len() < content.len()).then_some(replacement)
}

/// The raw text and original size an earlier envelope carries, when `content`
/// is one of ours. Trimming an envelope the trim itself wrote refines that
/// envelope rather than nesting a second one inside it: nesting would re-escape
/// the surviving text at every pass, so a trim called repeatedly on one message
/// would grow its provider payload without bound and the request preflight's
/// fixed point would never be reached. The partial is raw text, so it is
/// escaped exactly once per envelope and the original size stays the
/// original's.
fn prior_envelope(content: &str) -> Option<Prior> {
    if !content.starts_with(ENVELOPE_PREFIX) {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
    let object = parsed.as_object()?;
    let ours = object.get("truncated") == Some(&serde_json::Value::Bool(true))
        && object.keys().all(|key| {
            matches!(
                key.as_str(),
                "truncated" | "note" | "original_bytes" | "partial"
            )
        });
    if !ours {
        return None;
    }
    Some(Prior {
        partial: object
            .get("partial")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        original_bytes: object
            .get("original_bytes")
            .and_then(serde_json::Value::as_u64)
            .map_or(content.len(), |bytes| bytes as usize),
    })
}

struct Prior {
    partial: String,
    original_bytes: usize,
}

/// `text` as an envelope sized to `allowed` bytes, reporting `original_bytes`
/// as the size of the untruncated result. The tiers run from the widest — the
/// note, the original size and as much of the prefix as is left — down to the
/// flag alone, and the widest that fits is used. Every tier is valid JSON and
/// no tier is wider than `allowed` while `allowed` holds the smallest, so the
/// model receives a truthful cut result at every target rather than a document
/// cut mid-way or an envelope over the size it was given.
fn envelope(text: &str, original_bytes: usize, allowed: usize) -> String {
    let wide = format!(
        "{{\"truncated\":true,\"note\":\"{NOTE}\",\"original_bytes\":{original_bytes},\"partial\":\""
    );
    let lean = format!("{{\"truncated\":true,\"original_bytes\":{original_bytes},\"partial\":\"");
    for head in [&wide, &lean] {
        // `head` is everything before the partial; `"}` closes the envelope.
        let fixed = head.len() + 2;
        if allowed >= fixed {
            let partial = escaped_prefix(text, allowed - fixed);
            return format!("{head}{partial}\"}}");
        }
    }
    MINIMAL_ENVELOPE.to_owned()
}

/// The longest prefix of `text`, cut on a character boundary, whose escaped
/// JSON string body fits `capacity` bytes. Escaping can expand a character, so
/// the cut is found by pricing each character as it is walked — one pass, no
/// allocation, and never a split multi-byte character.
fn escaped_prefix(text: &str, capacity: usize) -> String {
    let mut used = 0usize;
    let mut end = 0usize;
    for ch in text.chars() {
        let cost = escaped_char_len(ch);
        if used + cost > capacity {
            break;
        }
        used += cost;
        end += ch.len_utf8();
    }
    json_string_body(&text[..end])
}

/// The bytes `ch` costs between the quotes of a JSON string. `serde_json`
/// writes `"`, `\` and the short escapes as two bytes and any other control
/// character as `\u00XX`; everything else is its own UTF-8 length. The
/// envelope's size bound rests on this matching `serde_json`, which
/// `escaped_char_len_matches_the_serializer` pins.
pub(super) fn escaped_char_len(ch: char) -> usize {
    match ch {
        '"' | '\\' | '\u{08}' | '\u{0c}' | '\n' | '\r' | '\t' => 2,
        c if (c as u32) < 0x20 => 6,
        c => c.len_utf8(),
    }
}

/// `text` as the body of a JSON string — escaped exactly once, so it can be
/// spliced into the envelope's `partial` without a second escaping pass.
fn json_string_body(text: &str) -> String {
    let quoted = serde_json::to_string(text).unwrap_or_default();
    quoted
        .get(1..quoted.len().saturating_sub(1))
        .unwrap_or_default()
        .to_owned()
}

/// The shape for non-JSON content: the longest prefix that leaves room for
/// the marker, cut on a character boundary. `None` when that is no shorter
/// than the content, so the caller's loop still makes progress.
fn marker_prefix(content: &str, allowed: usize) -> Option<String> {
    let bound = crate::loop_runner::tools::floor_boundary(
        content,
        allowed.saturating_sub(TRUNCATED_MARKER.len()),
    );
    (bound + TRUNCATED_MARKER.len() < content.len()).then(|| {
        let mut prefix = content[..bound].to_owned();
        prefix.push_str(TRUNCATED_MARKER);
        prefix
    })
}
