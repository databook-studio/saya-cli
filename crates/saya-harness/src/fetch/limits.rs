//! The explicit bounds of a fetch. A run states them once with its scope;
//! overrunning any of them is a typed error, never a silent short body.

use std::time::Duration;

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

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: MAX_TOTAL_BYTES,
            time_budget: TIME_BUDGET,
            max_redirect_hops: MAX_REDIRECT_HOPS,
        }
    }
}
