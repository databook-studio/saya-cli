//! The fetch capability: the fail-closed egress decision and the one tool
//! that acts on it.
//!
//! [`FetchPolicy`] is the decision — HTTPS only, refused address literals,
//! declared destinations — and [`HttpFetchTool`] is the bounded GET that may
//! only act through it: policy-judged URLs only, resolve-then-deny on every
//! returned address before anything connects, redirects re-judged per hop,
//! and content delivered to the model only as an untrusted [`ContextBlock`].
//! The wire itself sits behind [`FetchTransport`] so this decision logic can
//! be driven hermetically in tests; the production HTTP client
//! ([`ReqwestTransport`], no-follow redirects, all addresses resolved) and
//! the download machinery ([`http_download`]: streamed to the workspace, the
//! run budget, resumable partials) take [`FetchUrl`] as their input too, so
//! no fetch can exist that this module did not judge.
//!
//! The policy performs no network I/O itself; the tool runs
//! [`refuses_address`] for every address resolution returns for an accepted
//! host (resolve-then-deny, all returned IPs). Post-check rebinding (U5) is
//! the documented-unmitigated residual. The run's per-step executor member
//! ([`FetchTools`]) wires both tools into a run's toolset: the step's
//! policy, the run's shared transport and wallet, and the rendered
//! untrusted-block lane with its pre-bound and backstop (S2 decision 1).

pub mod adapter;
pub mod budget;
pub mod definition;
pub mod download;
mod download_error;
mod download_limits;
pub mod limits;
pub mod policy;
pub mod reqwest_transport;
mod resume;
mod session;
pub mod tools;
pub mod transport;

mod partial;

pub use adapter::FetchTools;
pub use budget::{DEFAULT_MAX_RUN_BYTES, DownloadBudget};
pub use definition::{http_download_definition, http_fetch_definition};
pub use download::http_download;
pub use download_error::{DownloadError, DownloadOutcome};
pub use download_limits::{DEFAULT_MAX_FILE_BYTES, DEFAULT_REQUEST_TIMEOUT, DownloadLimits};
pub use limits::{ENVELOPE_SLACK, FetchLimits};
pub use policy::{FetchDestination, FetchPolicy, FetchRefusal, FetchUrl, refuses_address};
pub use reqwest_transport::ReqwestTransport;
pub use tools::{FetchOutcome, FetchToolError, HttpFetchTool};
pub use transport::{FetchBody, FetchRequest, FetchTransport, FetchTransportError, WireResponse};
