//! The fetch capability: the fail-closed egress decision and the one tool
//! that acts on it.
//!
//! [`FetchPolicy`] is the decision — HTTPS only, refused address literals,
//! declared destinations — and [`HttpFetchTool`] is the bounded GET that may
//! only act through it: policy-judged URLs only, resolve-then-deny on every
//! returned address before anything connects, redirects re-judged per hop,
//! and content delivered to the model only as an untrusted [`ContextBlock`].
//! The wire itself sits behind [`FetchTransport`] so this decision logic can
//! be driven hermetically in tests; the production HTTP client and the
//! download machinery (streaming to the workspace, budget, resume) are
//! separate slices and will likewise take [`FetchUrl`] as their input, so no
//! fetch can exist that this module did not judge.
//!
//! The policy performs no network I/O itself; the tool runs
//! [`refuses_address`] for every address resolution returns for an accepted
//! host (resolve-then-deny, all returned IPs). Post-check rebinding (U5) is
//! the documented-unmitigated residual.

pub mod definition;
pub mod limits;
pub mod policy;
pub mod tools;
pub mod transport;

pub use definition::http_fetch_definition;
pub use limits::FetchLimits;
pub use policy::{FetchDestination, FetchPolicy, FetchRefusal, FetchUrl, refuses_address};
pub use tools::{FetchOutcome, FetchToolError, HttpFetchTool};
pub use transport::{FetchBody, FetchRequest, FetchTransport, FetchTransportError, WireResponse};
