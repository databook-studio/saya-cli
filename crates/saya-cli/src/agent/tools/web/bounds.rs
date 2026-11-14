//! The bounds one `web_fetch` call states, and the one it cannot raise. The
//! schema advertises exactly these numbers, so what the model reads and what
//! the tool enforces are the same figures.

use saya_agent::ToolError;
use saya_harness::fetch::FetchLimits;
use serde_json::Value;

/// The URL string's byte ceiling. Long enough for a real query string, short
/// enough that a URL is never the reason a prompt or a result is oversized.
pub(crate) const MAX_URL_BYTES: usize = 2048;

/// The text bound a call that states none gets: 24 KiB, comfortably inside one
/// context turn and far more than a documentation page's prose.
pub(crate) const DEFAULT_BODY_BYTES: usize = 24 * 1024;

/// The smallest text bound a call may state. Below this the answer is a
/// fragment, and a call that wants a fragment should not have spent a request
/// on it.
pub(crate) const MIN_BODY_BYTES: usize = 4 * 1024;

/// The largest text bound a call may state: the lane's own pre-bound — the
/// loop's tool-message cap minus the envelope slack — which is the same number
/// `http_fetch` runs under. A call cannot raise it: the body and its envelope
/// must fit the lane the result travels in, and a bound the lane would truncate
/// anyway would be a promise the tool could not keep.
pub(crate) fn max_body_bytes() -> usize {
    FetchLimits::for_tool_lane().max_total_bytes
}

/// The text bound one call asks for. `max_bytes` narrows and never widens:
/// absent means [`DEFAULT_BODY_BYTES`], and a value outside
/// [`MIN_BODY_BYTES`]..=[`max_body_bytes`] is refused typed rather than
/// clamped, so a call never believes it asked for something it did not.
pub(crate) fn body_cap(arguments: &Value) -> Result<usize, ToolError> {
    let Some(value) = arguments.get("max_bytes") else {
        return Ok(DEFAULT_BODY_BYTES);
    };
    let bound = value.as_u64().ok_or_else(|| {
        ToolError::Fetch("invalid tool arguments: max_bytes must be an integer".into())
    })?;
    let bound = usize::try_from(bound).unwrap_or(usize::MAX);
    if !(MIN_BODY_BYTES..=max_body_bytes()).contains(&bound) {
        return Err(ToolError::Fetch(format!(
            "invalid tool arguments: max_bytes must be between {MIN_BODY_BYTES} and {}",
            max_body_bytes()
        )));
    }
    Ok(bound)
}
