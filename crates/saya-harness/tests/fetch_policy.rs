//! The fetch-policy battery, written red-first against the policy module.
//!
//! The deliverable is the property: refusal of the private, loopback and
//! link-local space is pinned over the whole address space against a
//! reference predicate derived from the range contract — not against the
//! implementation — so a guard that only rejects enumerated examples fails
//! here. The explicit corpus covers the plan's adversarial list: the cloud
//! metadata endpoint, plain `http://`, decimal/hex/integer spellings of
//! private addresses, the `localhost` family, allowlist mismatch, redirect
//! chains ending private, and unparseable forms (refused, never guessed).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use proptest::prelude::*;
use saya_harness::fetch::{FetchDestination, FetchPolicy, FetchRefusal, refuses_address};

/// The declared destinations most tests run under: public hosts, a
/// documentation-range IPv4 and IPv6 — plus two *hostile* declarations (the
/// cloud metadata address, a plain-HTTP scheme) proving the declared list
/// can only ever narrow the policy, never widen it.
fn policy() -> FetchPolicy {
    FetchPolicy::new([
        FetchDestination::new("https", "example.com"),
        FetchDestination::new("https", "docs.example.org"),
        FetchDestination::new("https", "192.0.2.10"),
        FetchDestination::new("https", "[2001:db8::1]"),
        FetchDestination::new("http", "http-declared.example"),
        FetchDestination::new("https", "169.254.169.254"),
        FetchDestination::new("https", "localhost"),
    ])
}

fn must_refuse(url: &str) -> FetchRefusal {
    let policy = policy();
    let Err(refusal) = policy.allow_url(url) else {
        panic!("`{url}` must be refused");
    };
    refusal
}

fn refuses_v4(ip: Ipv4Addr) -> bool {
    refuses_address(IpAddr::V4(ip)).is_err()
}

fn refuses_v6(ip: Ipv6Addr) -> bool {
    refuses_address(IpAddr::V6(ip)).is_err()
}

// --- the three gates, explicit -------------------------------------------

// --- redirects: same policy, every hop ------------------------------------

// --- the U5 residual, asserted not solved ----------------------------------

// --- boundaries ------------------------------------------------------------

// The session policy (U1): any HTTPS host outside the refused ranges, with
// the per-call ask replacing the declared-destination gate. The structural
// gates stay absolute.
// ---------------------------------------------------------------------------

#[path = "fetch_policy/url_policy.rs"]
mod url_policy;

#[path = "fetch_policy/session_policy.rs"]
mod session_policy;

#[path = "fetch_policy/address_boundaries.rs"]
mod address_boundaries;
