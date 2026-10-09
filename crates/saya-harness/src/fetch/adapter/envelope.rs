//! The fetch result's honest envelope: the rendered untrusted block inside
//! a JSON wrapper, delivered under the loop's tool-message cap. This is the
//! invariant that remains: an unclosed block is never emitted.

use saya_agent::{ContextBlock, render_untrusted_block};
use serde_json::Value;

/// Builds the fetch result's envelope — `{"url": <final url>, "content":
/// <the rendered untrusted block>}` — under the loop's tool-message cap.
/// The normal path is the pre-bound: the wired `max_total_bytes` sits below
/// the cap minus envelope slack, so the first render fits and `truncated`
/// stays false by construction (a success is always the whole body). The
/// defensive backstop covers the pathological remainder — a hostile label,
/// escaping growth, a tighter cap: the block is flagged `truncated` so the
/// block's own marker renders inside the delimiters, and the *body* is cut
/// at a char boundary and re-rendered until the envelope fits. The floor —
/// the envelope with an empty body — is measured first, and the allowance
/// converges geometrically on any measured overshoot, so escaping and JSON
/// growth of any factor still terminate at a closed block. `None` means
/// even the empty-body block cannot fit (an absurd final URL): the caller
/// fails the call typed rather than emitting an unclosed block.
pub(super) fn bounded_fetch_envelope(
    url: &str,
    block: &mut ContextBlock,
    cap: usize,
) -> Option<Value> {
    let build = |block: &ContextBlock| serde_json::json!({ "url": url, "content": render_untrusted_block(block) });
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
