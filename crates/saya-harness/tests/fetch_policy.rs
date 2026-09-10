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

/// Gate 1: HTTPS only. Plain HTTP is refused even when the host is declared,
/// and even when an `http` destination was declared for the run.
#[test]
fn refuses_plain_http_even_when_the_host_is_declared() {
    assert!(matches!(
        must_refuse("http://example.com/"),
        FetchRefusal::NonHttpsScheme { scheme } if scheme == "http"
    ));
    assert!(matches!(
        must_refuse("HTTP://example.com/"),
        FetchRefusal::NonHttpsScheme { .. }
    ));
    assert!(matches!(
        must_refuse("ftp://example.com/pub/"),
        FetchRefusal::NonHttpsScheme { scheme } if scheme == "ftp"
    ));
    assert!(matches!(
        must_refuse("file:///etc/passwd"),
        FetchRefusal::NonHttpsScheme { .. }
    ));
}

/// Gate 2: the cloud metadata endpoint is refused whether declared or not —
/// range refusal outranks the allowlist.
#[test]
fn refuses_the_cloud_metadata_endpoint_declared_or_not() {
    let bare = FetchPolicy::new([]);
    assert!(matches!(
        bare.allow_url("https://169.254.169.254/latest/meta-data/"),
        Err(FetchRefusal::DisallowedAddress { host }) if host == "169.254.169.254"
    ));
    assert!(matches!(
        must_refuse("https://169.254.169.254/latest/meta-data/"),
        FetchRefusal::DisallowedAddress { .. }
    ));
    assert!(matches!(
        must_refuse("https://169.254.0.1/"),
        FetchRefusal::DisallowedAddress { .. }
    ));
    assert!(matches!(
        must_refuse("https://169.254.254.254/"),
        FetchRefusal::DisallowedAddress { .. }
    ));
}

/// Gate 2 over the named ranges.
#[test]
fn refuses_private_loopback_and_link_local_literals() {
    for url in [
        "https://10.1.2.3/",
        "https://172.16.0.1/",
        "https://172.31.255.254/",
        "https://192.168.0.1/",
        "https://127.0.0.1/",
        "https://0.0.0.0/",
        "https://[::1]/",
        "https://[fd00::1]/",
        "https://[fe80::1]/",
    ] {
        assert!(
            matches!(must_refuse(url), FetchRefusal::DisallowedAddress { .. }),
            "{url}"
        );
    }
}

/// Gate 2 must survive spelling: the URL parser canonicalizes decimal, hex,
/// octal and short integer host forms to IPv4, and the policy judges the
/// address it will actually contact, not the string the plan wrote.
#[test]
fn refuses_non_canonical_spellings_of_private_addresses() {
    for url in [
        "https://2130706433/",                 // 127.0.0.1 as one decimal integer
        "https://0x7f000001/",                 // 127.0.0.1 as one hex integer
        "https://0177.0.0.1/",                 // 127.0.0.1 with an octal octet
        "https://127.1/",                      // 127.0.0.1 as a short quad
        "https://2852039166/",                 // 169.254.169.254 as one decimal integer
        "https://0xa9fea9fe/",                 // 169.254.169.254 as one hex integer
        "https://[::ffff:10.0.0.1]/",          // IPv4-mapped IPv6 carrying RFC 1918
        "https://[::127.0.0.1]/",              // deprecated IPv4-compatible form
        "https://[64:ff9b::169.254.169.254]/", // NAT64 form of the metadata endpoint
    ] {
        assert!(
            matches!(must_refuse(url), FetchRefusal::DisallowedAddress { .. }),
            "{url}"
        );
    }
}

/// `localhost` and every subdomain of it are loopback by name (RFC 6761),
/// declared or not, in any case or trailing-dot spelling.
#[test]
fn refuses_the_localhost_family_by_name() {
    for url in [
        "https://localhost/",
        "https://sub.localhost/",
        "https://deep.sub.localhost/",
        "https://LOCALHOST/",
        "https://localhost./",
    ] {
        assert!(
            matches!(must_refuse(url), FetchRefusal::DisallowedAddress { .. }),
            "{url}"
        );
    }
}

/// Gate 3: the host must be declared, exactly — no suffix or prefix games,
/// no subdomain inheritance, no trailing-dot FQDN variant.
#[test]
fn refuses_hosts_outside_the_declared_destinations() {
    for url in [
        "https://evil.example.net/",
        "https://example.com.evil.com/",
        "https://badexample.com/",
        "https://sub.example.com/",
        "https://example.com./",
        "https://198.51.100.7/",
    ] {
        assert!(
            matches!(must_refuse(url), FetchRefusal::UndeclaredDestination { .. }),
            "{url}"
        );
    }
}

/// Anything that does not parse into an absolute URL with a host is
/// refused — never guessed at.
#[test]
fn refuses_urls_that_do_not_parse_rather_than_guessing() {
    for url in [
        "not a url",
        "https://",
        "https://1.2.3.4.5/",
        "/relative/path",
    ] {
        let refusal = must_refuse(url);
        assert!(
            matches!(refusal, FetchRefusal::MalformedUrl),
            "{url}: {refusal}"
        );
    }
}

/// The legitimate path: declared public hosts are allowed, normalized
/// (case), and ports are currently unpinned — a documented, narrower-later
/// choice, pinned here so a change to it is a decision, not drift.
#[test]
fn allows_declared_public_destinations() {
    let policy = policy();
    for (url, canonical) in [
        (
            "https://example.com/path?q=1",
            "https://example.com/path?q=1",
        ),
        ("https://EXAMPLE.com/", "https://example.com/"),
        ("https://docs.example.org", "https://docs.example.org/"),
        ("https://192.0.2.10/", "https://192.0.2.10/"),
        ("https://[2001:db8::1]/", "https://[2001:db8::1]/"),
        ("https://example.com:8443/", "https://example.com:8443/"),
    ] {
        let allowed = policy
            .allow_url(url)
            .unwrap_or_else(|refusal| panic!("{url} must be allowed, got {refusal}"));
        assert_eq!(allowed.as_str(), canonical);
    }
}

// --- redirects: same policy, every hop ------------------------------------

/// A redirect is re-judged in full at each hop. Two hops pass, the third
/// names the metadata endpoint, and it is refused there — nothing about the
/// earlier hops carries trust forward.
#[test]
fn redirect_chains_ending_in_a_private_target_are_refused_at_that_hop() {
    let policy = policy();
    let first = policy
        .allow_url("https://example.com/start")
        .expect("declared host");
    let second = policy
        .allow_redirect(&first, "/hop")
        .expect("relative redirect to the same declared host");
    assert_eq!(second.as_str(), "https://example.com/hop");
    assert!(matches!(
        policy.allow_redirect(&second, "https://169.254.169.254/latest/meta-data/"),
        Err(FetchRefusal::DisallowedAddress { .. })
    ));
}

/// Every gate applies to the redirect target on its own: scheme, range, and
/// allowlist — including scheme-relative `Location` values, which inherit
/// the scheme but not the verdict.
#[test]
fn redirect_targets_fail_closed_on_every_gate() {
    let policy = policy();
    let base = policy
        .allow_url("https://example.com/a")
        .expect("declared host");
    assert!(matches!(
        policy.allow_redirect(&base, "http://example.com/b"),
        Err(FetchRefusal::NonHttpsScheme { .. })
    ));
    assert!(matches!(
        policy.allow_redirect(&base, "https://127.0.0.1:9200/"),
        Err(FetchRefusal::DisallowedAddress { .. })
    ));
    assert!(matches!(
        policy.allow_redirect(&base, "//10.0.0.5/x"),
        Err(FetchRefusal::DisallowedAddress { .. })
    ));
    assert!(matches!(
        policy.allow_redirect(&base, "https://undeclared.example.net/"),
        Err(FetchRefusal::UndeclaredDestination { .. })
    ));
}

// --- the U5 residual, asserted not solved ----------------------------------

/// A DNS name is not resolved by the pure policy: a declared name passes the
/// URL-level decision by shape alone. Range enforcement for names happens
/// where resolution happens — the resolver side must run `refuses_address`
/// on *every* returned address. Post-check rebinding remains the
/// documented-unmitigated residual (plan §2 U5); this test pins that the
/// limitation is stated in behavior, not hand-waved away.
#[test]
fn a_declared_dns_name_passes_the_url_policy_without_resolution() {
    let policy = FetchPolicy::new([FetchDestination::new("https", "metadata.internal")]);
    let allowed = policy
        .allow_url("https://metadata.internal/keys")
        .expect("declared name passes the URL policy; resolution owns the IP check");
    assert_eq!(allowed.host(), Some("metadata.internal"));
    // The seam the resolver side composes with: had the name resolved to a
    // private address, this predicate is what refuses it — no network I/O
    // here, the check is pure.
    assert!(refuses_v4(Ipv4Addr::new(10, 0, 0, 7)));
}

// --- boundaries ------------------------------------------------------------

/// Refused IPv4 ranges, straight from the contract.
const REFUSED_V4_RANGES: &[(Ipv4Addr, Ipv4Addr)] = &[
    (Ipv4Addr::new(0, 0, 0, 0), Ipv4Addr::new(0, 255, 255, 255)),
    (Ipv4Addr::new(10, 0, 0, 0), Ipv4Addr::new(10, 255, 255, 255)),
    (
        Ipv4Addr::new(127, 0, 0, 0),
        Ipv4Addr::new(127, 255, 255, 255),
    ),
    (
        Ipv4Addr::new(169, 254, 0, 0),
        Ipv4Addr::new(169, 254, 255, 255),
    ),
    (
        Ipv4Addr::new(172, 16, 0, 0),
        Ipv4Addr::new(172, 31, 255, 255),
    ),
    (
        Ipv4Addr::new(192, 168, 0, 0),
        Ipv4Addr::new(192, 168, 255, 255),
    ),
];

fn expected_refuses_v4(ip: Ipv4Addr) -> bool {
    let n = u32::from(ip);
    REFUSED_V4_RANGES
        .iter()
        .any(|&(low, high)| (u32::from(low)..=u32::from(high)).contains(&n))
}

#[test]
fn refused_ipv4_ranges_hold_at_every_boundary() {
    for &(low, high) in REFUSED_V4_RANGES {
        let (low, high) = (u32::from(low), u32::from(high));
        assert!(
            refuses_v4(Ipv4Addr::from(low)),
            "{low} (range floor) must be refused"
        );
        assert!(
            refuses_v4(Ipv4Addr::from(high)),
            "{high} (range ceiling) must be refused"
        );
        if low > 0 {
            assert!(
                !refuses_v4(Ipv4Addr::from(low - 1)),
                "{} (below floor) must be allowed",
                low - 1
            );
        }
        if high < u32::MAX {
            assert!(
                !refuses_v4(Ipv4Addr::from(high + 1)),
                "{} (above ceiling) must be allowed",
                high + 1
            );
        }
    }
}

#[test]
fn refused_ipv6_and_embedded_forms_hold_at_the_boundaries() {
    for ip in [
        Ipv6Addr::LOCALHOST,
        Ipv6Addr::UNSPECIFIED,
        Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1),
        Ipv6Addr::new(
            0xfdff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff,
        ),
        Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
        Ipv6Addr::new(
            0xfebf, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff,
        ),
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001), // ::ffff:10.0.0.1
        Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0x7f00, 0x0001),      // ::127.0.0.1
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xa9fe, 0xa9fe), // ::ffff:169.254.169.254
        Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, 0xa9fe, 0xa9fe), // 64:ff9b::169.254.169.254
    ] {
        assert!(refuses_v6(ip), "{ip} must be refused");
    }
    for ip in [
        Ipv6Addr::new(
            0xfbff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff,
        ),
        Ipv6Addr::new(0xfe00, 0, 0, 0, 0, 0, 0, 1),
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1),
        Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0808, 0x0808), // ::ffff:8.8.8.8
    ] {
        assert!(!refuses_v6(ip), "{ip} must be allowed");
    }
}

// --- the properties: the deliverable ----------------------------------------

/// Reference predicate for IPv6, derived from the range contract (not from
/// the implementation): loopback, unspecified, unique-local `fc00::/7`,
/// link-local `fe80::/10`, and every embedded-IPv4 form — mapped
/// `::ffff:0:0/96`, the deprecated compatible `::/96`, and NAT64
/// `64:ff9b::/96` — which inherits the IPv4 refusal.
fn expected_refuses_v6(ip: Ipv6Addr) -> bool {
    let o = ip.octets();
    if ip == Ipv6Addr::LOCALHOST || ip == Ipv6Addr::UNSPECIFIED {
        return true;
    }
    if o[0] == 0xfc || o[0] == 0xfd {
        return true;
    }
    if o[0] == 0xfe && (o[1] & 0xc0) == 0x80 {
        return true;
    }
    let mapped = o[..10] == [0u8; 10] && o[10] == 0xff && o[11] == 0xff;
    let compatible = o[..12] == [0u8; 12];
    let nat64 =
        o[0] == 0x00 && o[1] == 0x64 && o[2] == 0xff && o[3] == 0x9b && o[4..12] == [0u8; 8];
    (mapped || compatible || nat64)
        && expected_refuses_v4(Ipv4Addr::new(o[12], o[13], o[14], o[15]))
}

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(4096))]

    /// Every IPv4 address is refused exactly when it sits in a refused
    /// range — sampled across the whole 32-bit space — and the full URL
    /// decision agrees: with the address itself declared, range refusal
    /// still wins; with it public and declared, the fetch is allowed.
    #[test]
    fn every_ipv4_address_is_refused_exactly_when_in_a_refused_range(n in any::<u32>()) {
        let ip = Ipv4Addr::from(n);
        let expected = expected_refuses_v4(ip);

        prop_assert_eq!(refuses_v4(ip), expected);

        let url = format!("https://{ip}/");
        let policy = FetchPolicy::new([FetchDestination::new("https", &ip.to_string())]);
        if expected {
            let refused = matches!(
                policy.allow_url(&url),
                Err(FetchRefusal::DisallowedAddress { .. })
            );
            prop_assert!(refused);
        } else {
            prop_assert!(policy.allow_url(&url).is_ok());
        }
    }

    /// The same property across the IPv6 space, sampled over all 128 bits,
    /// composed through the URL decision with the bracketed literal
    /// declared.
    #[test]
    fn every_ipv6_address_is_refused_exactly_when_in_a_refused_range(parts in any::<[u16; 8]>()) {
        let ip = Ipv6Addr::from(parts);
        let expected = expected_refuses_v6(ip);

        prop_assert_eq!(refuses_v6(ip), expected);

        let url = format!("https://[{ip}]/");
        let policy = FetchPolicy::new([FetchDestination::new("https", &format!("[{ip}]"))]);
        if expected {
            let refused = matches!(
                policy.allow_url(&url),
                Err(FetchRefusal::DisallowedAddress { .. })
            );
            prop_assert!(refused);
        } else {
            prop_assert!(policy.allow_url(&url).is_ok());
        }
    }
}
