//! The explicit bounds of a fetch. A run states them once with its scope;
//! overrunning any of them is a typed error, never a silent short body.

use std::time::Duration;

use saya_agent::{AgentLimits, tool_message_cap};

/// The byte bound: the body's destination is the model's context, and 1 MiB
/// of text is already far beyond what one useful turn can carry. Anything a
/// run needs beyond this belongs to the download slice's bounded stream to
/// the workspace, not in context — so the bound trips as a typed error
/// rather than silently shortening the body.
pub const MAX_TOTAL_BYTES: usize = 1024 * 1024;

/// The wall-clock bound for one fetch. DNS, every redirect hop, and the body
/// read share it: a stalled name or endpoint is cut off at the deadline
/// rather than waited on. Generous enough for a slow host; tight enough that
/// a fetch cannot dominate a run turn, which has its own budget.
pub const TIME_BUDGET: Duration = Duration::from_secs(30);

/// The redirect bound, matching curl's long-standing default: room for a
/// legitimate chain, cheap to trip on a loop (every hop is re-judged by the
/// policy and re-resolved by the tool).
pub const MAX_REDIRECT_HOPS: usize = 5;

/// Slack reserved below the loop's tool-message cap for everything a fetch
/// result carries besides the body: the JSON envelope, the (policy-judged)
/// final URL, the block's wrapper lines and label, and short-string
/// escaping growth. The lane's body bound is the cap minus this slack, so
/// an ordinary fetch success fits the lane with room to spare and the
/// loop's own truncation marker never fires on one.
pub const ENVELOPE_SLACK: usize = 4_096;

/// The bounds of one fetch, stated rather than implied.
#[derive(Clone, Copy, Debug)]
pub struct FetchLimits {
    /// Maximum body bytes read across the whole fetch.
    pub max_total_bytes: usize,
    /// Wall-clock budget shared by DNS, all hops, and the body read.
    pub time_budget: Duration,
    /// Maximum redirects followed, each re-judged by the policy.
    pub max_redirect_hops: usize,
}

impl FetchLimits {
    /// The tool-message cap the lane pre-binds under — the same number the
    /// loop's `tool_message` truncates at (`min(byte_budget, 65_536)`;
    /// runs pin the default conversation budget). Exported through this
    /// constructor so the harness's declared fetch bound and the loop's
    /// truncation point are the same number by construction: the bound is
    /// the cap minus [`ENVELOPE_SLACK`], and if runs ever thread a
    /// configured context budget instead of the default, this derivation
    /// follows in one line.
    pub fn tool_lane_cap() -> usize {
        tool_message_cap(AgentLimits::default().context_byte_budget)
    }

    /// The bounds the loop-admitted `http_fetch` runs under. A body over
    /// `max_total_bytes` is the tool's own typed `BodyTooLarge` — never a
    /// short success — and a success at this bound always fits the loop's
    /// tool-message cap, so `bounded_json` truncation (which appends its
    /// marker *outside* any block structure and would orphan the block's
    /// closing sentinel) never fires on a fetch result. The adapter's
    /// backstop covers the pathological remainder (a hostile label, escaping
    /// growth, a tighter cap).
    pub fn for_tool_lane() -> Self {
        Self {
            max_total_bytes: Self::tool_lane_cap() - ENVELOPE_SLACK,
            time_budget: TIME_BUDGET,
            max_redirect_hops: MAX_REDIRECT_HOPS,
        }
    }
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: MAX_TOTAL_BYTES,
            time_budget: TIME_BUDGET,
            max_redirect_hops: MAX_REDIRECT_HOPS,
        }
    }
}
