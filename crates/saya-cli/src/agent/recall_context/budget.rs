//! Bounding the rendered context block to what fits the agent message budget.
//!
//! The byte bound is on the **rendered** block the request actually sends — not
//! the serialized claim payloads `recall` selected — and against what is left of
//! the agent message budget after the system prompt and the user's own question.
//! The question is the point and the context is the assist, so context never
//! consumes the bytes the prompt needs: [`bound_body`] drops contracts from the
//! end (least-relevant first, since `recall` ranks them) until the rendered block
//! fits, and if even the first contract does not fit it is omitted and the block
//! is marked truncated.
//!
//! The size is measured with [`saya_agent::turn_bytes`] — the exact post-escape,
//! post-wrapper size [`saya_agent::build_messages`] enforces — so the pre-build
//! bound and the build share one accounting and cannot drift.

use crate::contracts::RetrievedContract;
use saya_agent::{ContextBlock, MAX_HISTORY_BYTES, turn_bytes};
use std::collections::HashMap;

use super::BLOCK_LABEL;

/// Renders the largest prefix of `contracts` whose rendered block fits both the
/// configured `max_bytes` cap on the body and the agent message budget (what is
/// left of [`MAX_HISTORY_BYTES`] after the system prompt and the user's prompt).
///
/// The byte bound is on the rendered block — the body plus the wrapper
/// [`turn_bytes`] adds (preamble, delimiters, the `source:` line, the truncation
/// marker, escaping) — measured with the same accounting `build_messages`
/// enforces, so the two cannot drift. `truncated: true` is reserved while
/// searching (the worst-case wrapper, with the marker), so the block fits whether
/// or not it ends up truncated. Contracts are dropped from the end — least-relevant
/// first, since `recall` ranks them — until the block fits.
///
/// Returns `(body, truncated)`: the rendered body of the fitting prefix, and
/// whether the byte bound dropped any contract. If even the first contract does
/// not fit, the body is empty and `truncated` is true — the caller surfaces that
/// as a truncated empty block rather than silence, so the model learns recall
/// happened and the context was too large to include.
pub(super) fn bound_body(
    contracts: &[RetrievedContract],
    name_of: &HashMap<String, String>,
    system_prompt: Option<&str>,
    prompt: &str,
    max_body_bytes: usize,
) -> (String, bool) {
    // Find the largest prefix k whose rendered block fits both the configured
    // body cap and the agent message budget. Both constraints are monotonic in k
    // (a longer body only grows), so the largest fitting k is well-defined.
    for k in (1..=contracts.len()).rev() {
        let body = super::render::render_body(&contracts[..k], name_of);
        if body.len() <= max_body_bytes && fits_message_budget(system_prompt, &body, prompt) {
            return (body, k < contracts.len());
        }
    }
    // Even the first contract does not fit: omit every claim, mark truncation.
    (String::new(), true)
}

/// Whether a block with this body, alongside the system prompt and the user's
/// prompt, still fits under the agent message budget. Uses [`turn_bytes`] — the
/// exact post-escape, post-wrapper size `build_messages` enforces — so the
/// pre-build check and the build share one accounting. The block is measured with
/// `truncated: true` (the worst-case wrapper that includes the truncation marker),
/// so a block that fits here fits whether or not it is ultimately marked truncated.
fn fits_message_budget(system_prompt: Option<&str>, body: &str, prompt: &str) -> bool {
    let block = ContextBlock {
        label: BLOCK_LABEL.to_string(),
        body: body.to_string(),
        truncated: true,
    };
    turn_bytes(system_prompt, std::slice::from_ref(&block), prompt) <= MAX_HISTORY_BYTES
}
