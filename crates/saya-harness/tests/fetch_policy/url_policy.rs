use super::*;

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
