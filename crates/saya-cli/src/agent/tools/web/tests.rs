//! The `web_fetch` battery, in four parts:
//!
//! - `advertisement` — what the schema says, which modes advertise the tool at
//!   all, and that `[tools] allow` can take it away.
//! - `consent` — the consent rule: who is asked, once per call, and what a call
//!   earns when nobody can be.
//! - `egress` — the bounds and the refusals: an address the policy refuses
//!   never reaches the wire, a page over the call's bound comes back cut with
//!   the elision marker, and a result names what it fetched.
//! - `conversion` — HTML rendered as text, and everything else passed through.
//!
//! The wire is the hermetic [`fixture`](super::fixture): no test here opens a
//! socket.

#[path = "tests/advertisement.rs"]
mod advertisement;
#[path = "tests/consent.rs"]
mod consent;
#[path = "tests/conversion.rs"]
mod conversion;
#[path = "tests/egress.rs"]
mod egress;
