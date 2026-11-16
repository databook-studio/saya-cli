//! The fetch result's honest envelope: the rendered untrusted block inside
//! a JSON wrapper, delivered under the loop's tool-message cap. This is the
//! invariant that remains: an unclosed block is never emitted.

use saya_agent::{ContextBlock, render_untrusted_block};
use serde_json::{Map, Value};

/// Builds the `http_fetch` result's envelope — `{"url": <final url>,
/// "content": <the rendered untrusted block>}` — under the loop's
/// tool-message cap. A thin wrapper over [`bounded_block_envelope`], which
/// carries the whole rule; this is the one shape `http_fetch` has always
/// had.
pub(super) fn bounded_fetch_envelope(
    url: &str,
    block: &mut ContextBlock,
    cap: usize,
) -> Option<Value> {
    let mut fields = Map::new();
    fields.insert("url".to_owned(), Value::String(url.to_owned()));
    bounded_block_envelope(fields, block, cap)
}

/// Builds a fetch-shaped result — `fields` plus `"content": <the rendered
/// untrusted block>` — under the loop's tool-message cap. The normal path is
/// the pre-bound: the wired body bound sits below the cap minus envelope
/// slack, so the first render fits and `truncated` stays false by
/// construction. The defensive backstop covers the pathological remainder —
/// a hostile label, escaping growth, a tighter cap: the block is flagged
/// `truncated` so the block's own marker renders inside the delimiters, and
/// the *body* is cut at a char boundary and re-rendered until the envelope
/// fits. The floor — the envelope with an empty body — is measured first,
/// and the allowance converges geometrically on any measured overshoot, so
/// escaping and JSON growth of any factor still terminate at a closed block.
/// `None` means even the empty-body block cannot fit (an absurd field value):
/// the caller fails the call typed rather than emitting an unclosed block.
pub fn bounded_block_envelope(
    fields: Map<String, Value>,
    block: &mut ContextBlock,
    cap: usize,
) -> Option<Value> {
    let build = |block: &ContextBlock| {
        let mut object = fields.clone();
        object.insert(
            "content".to_owned(),
            Value::String(render_untrusted_block(block)),
        );
        Value::Object(object)
    };
    let full = build(block);
    if full.to_string().len() <= cap {
        return Some(full);
    }
    // Every backstop render carries the in-block truncation marker, so the
    // floor — everything the envelope needs besides body bytes — includes it.
    block.truncated = true;
    let empty = build(&ContextBlock {
        label: block.label.clone(),
        body: String::new(),
        truncated: true,
    });
    let floor = empty.to_string().len();
    if floor > cap {
        return None;
    }
    // The rendered body can cost several raw bytes per character (the
    // escape scheme doubles backslashes, the JSON string doubles them
    // again), so start at a quarter of the room and converge geometrically:
    // the allowance reaches the floor before it reaches zero, and the loop
    // terminates with a fitting, closed block.
    let mut allowance = (cap - floor) / 4;
    loop {
        block
            .body
            .truncate(floor_char_boundary(&block.body, allowance));
        let candidate = build(block);
        if candidate.to_string().len() <= cap {
            return Some(candidate);
        }
        if allowance == 0 {
            return None;
        }
        allowance /= 2;
    }
}

/// Largest byte index `<= idx` on a UTF-8 character boundary, so a
/// backstop cut never splits a multi-byte sequence.
/// (`str::floor_char_boundary` is stable only since 1.91, above the MSRV.)
fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        idx = text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}
