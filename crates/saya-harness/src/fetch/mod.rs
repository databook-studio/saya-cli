//! The fetch policy: the fail-closed egress decision for a run. This slice
//! is the decision only — the `http_fetch` tool and the download machinery
//! are separate slices and must take [`FetchUrl`] as their input, so no
//! fetch can exist that this module did not judge.
//!
//! No network I/O lives here. DNS names are not resolved; the resolver side
//! must run [`FetchPolicy::address_refused`] on every address resolution
//! returns for an accepted host (resolve-then-deny, all returned IPs). Until
//! that call exists, a declared name that resolves private is uncaught — the
//! U5 post-check rebinding residual, documented-unmitigated.

pub mod policy;

pub use policy::{FetchDestination, FetchPolicy, FetchRefusal, FetchUrl, refuses_address};
