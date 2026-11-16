use super::*;

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

// ---------------------------------------------------------------------------
