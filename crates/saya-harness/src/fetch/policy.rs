//! The fetch egress policy: the fail-closed decision of whether a URL may
//! be fetched, judged against a run's declared destinations. Three gates,
//! all required, in order:
//!
//! 1. **HTTPS only.** Any other scheme — plain `http` included — is refused
//!    even when declared. The declared list can narrow this policy, never
//!    widen it.
//! 2. **Address-literal refusal.** A host that is an IP literal in any
//!    spelling the URL parser accepts (dotted quad, decimal/hex/octal
//!    integers, bracketed IPv6) is refused in IPv4 `0/8`, `10/8`, `127/8`,
//!    `169.254/16` (the cloud metadata endpoint `169.254.169.254`),
//!    `172.16/12`, `192.168/16`; IPv6 loopback `::1`, unspecified `::`,
//!    unique-local `fc00::/7`, link-local `fe80::/10`, and every
//!    embedded-IPv4 form (`::ffff:0:0/96` mapped, the deprecated `::/96`
//!    compatible form, `64:ff9b::/96` NAT64), which inherit the IPv4 rules.
//!    `localhost` and its subdomains are refused by name (RFC 6761).
//! 3. **Declared destinations.** The parsed scheme + host must be one the
//!    run declared and the user approved.
//!
//! Residual, stated not silent: no DNS resolution happens here (no network
//! I/O in this module), so a declared *name* that resolves private is not
//! caught at the URL level. The resolver side must run [`refuses_address`]
//! for every address resolution returns for an accepted host —
//! resolve-then-deny over all returned IPs. Post-check rebinding (U5) is
//! the documented-unmitigated residual.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;
use url::{Host, Url};

/// A destination a run declares and the user approves: one scheme + host.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FetchDestination {
    scheme: String,
    host: String,
}

impl FetchDestination {
    /// Declares a destination. Matching is exact against the host the URL
    /// parser produces (already lowercased; IPv6 literals keep their
    /// brackets), so `example.com` does not cover `example.com.`.
    pub fn new(scheme: &str, host: &str) -> Self {
        Self {
            scheme: scheme.to_ascii_lowercase(),
            host: host.to_ascii_lowercase(),
        }
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

/// Why a fetch was refused. Fails closed: every variant is terminal, never
/// a request to retry with a relaxed rule.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum FetchRefusal {
    /// The string does not parse as an absolute URL, or names no host.
    #[error("fetch URL is malformed; refused, never guessed at")]
    MalformedUrl,

    /// The scheme is not HTTPS — plain HTTP included — declared or not.
    #[error("fetch scheme `{scheme}` is refused: only https is permitted")]
    NonHttpsScheme { scheme: String },

    /// The host is an address literal in a refused range, or a loopback
    /// name — declared or not.
    #[error(
        "fetch host `{host}` sits in a refused range (private, loopback, link-local, unique-local)"
    )]
    DisallowedAddress { host: String },

    /// The host was never declared for this run.
    #[error("fetch host `{host}` is not a destination declared for this run")]
    UndeclaredDestination { host: String },
}

/// A URL the policy accepted. Constructible only through a [`FetchPolicy`],
/// so the fetch tool and downloader (later slices) can take this type and
/// cannot fetch anything this module did not judge.
#[derive(Clone, Debug)]
pub struct FetchUrl(Url);

impl FetchUrl {
    /// The canonical form of the accepted URL, as the fetcher will use it.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The accepted host, as parsed (IPv6 literals keep their brackets).
    pub fn host(&self) -> Option<&str> {
        self.0.host_str()
    }
}

/// The per-run egress decision over declared destinations.
#[derive(Clone, Debug)]
pub struct FetchPolicy {
    destinations: BTreeSet<FetchDestination>,
    /// The session shape: any HTTPS host outside the refused ranges may be
    /// fetched, the per-call ask being the destination consent. Gates 1 and
    /// 2 are unchanged — the structural refusals stay absolute — only the
    /// declared-destination gate is waived, and only for a surface that asks
    /// before every fetch.
    session_wide: bool,
}

impl FetchPolicy {
    /// Builds the policy from the run's declared destinations.
    pub fn new(destinations: impl IntoIterator<Item = FetchDestination>) -> Self {
        Self {
            destinations: destinations.into_iter().collect(),
            session_wide: false,
        }
    }

    /// The interactive session's policy: HTTPS only, refused ranges still
    /// refused, and every host consented per call instead of declared up
    /// front. A run never uses this shape — its destinations are declared
    /// with its plan and enforced by this policy.
    pub fn session() -> Self {
        Self {
            destinations: Default::default(),
            session_wide: true,
        }
    }

    /// The decision: may this URL be fetched? The first hop of every fetch
    /// passes through here.
    pub fn allow_url(&self, url: &str) -> Result<FetchUrl, FetchRefusal> {
        let parsed = Url::parse(url).map_err(|_| FetchRefusal::MalformedUrl)?;
        self.allow_parsed(parsed)
    }

    /// Judges a redirect target with the full policy — the same gates, no
    /// trust carried from the hop that produced it. `location` may be
    /// relative; it resolves against the already-validated `from`, and the
    /// absolute result is judged as a fresh URL.
    pub fn allow_redirect(
        &self,
        from: &FetchUrl,
        location: &str,
    ) -> Result<FetchUrl, FetchRefusal> {
        let target = from
            .0
            .join(location)
            .map_err(|_| FetchRefusal::MalformedUrl)?;
        self.allow_parsed(target)
    }

    fn allow_parsed(&self, url: Url) -> Result<FetchUrl, FetchRefusal> {
        if url.scheme() != "https" {
            return Err(FetchRefusal::NonHttpsScheme {
                scheme: url.scheme().to_string(),
            });
        }
        let refused = match url.host() {
            Some(Host::Ipv4(v4)) => refuses_v4(v4),
            Some(Host::Ipv6(v6)) => refuses_v6(v6),
            Some(Host::Domain(domain)) => is_loopback_name(domain),
            None => return Err(FetchRefusal::MalformedUrl),
        };
        let host = url.host_str().unwrap_or_default().to_string();
        if refused {
            return Err(FetchRefusal::DisallowedAddress { host });
        }
        let destination = FetchDestination::new(url.scheme(), &host);
        if !self.session_wide && !self.destinations.contains(&destination) {
            return Err(FetchRefusal::UndeclaredDestination { host });
        }
        Ok(FetchUrl(url))
    }
}

/// The refused IPv4 space: this-network, RFC 1918, loopback, and link-local
/// (the cloud metadata endpoint's range).
fn refuses_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 0
        || a == 10
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (b & 0xf0) == 0x10)
        || (a == 192 && b == 0xa8)
}

/// The refused IPv6 space: loopback, unspecified, unique-local, link-local,
/// and every form that embeds an IPv4 address, which inherits the IPv4
/// refusal.
fn refuses_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    let o = ip.octets();
    if o[0] == 0xfc || o[0] == 0xfd || (o[0] == 0xfe && (o[1] & 0xc0) == 0x80) {
        return true;
    }
    let mapped = o[..10].iter().all(|byte| *byte == 0) && o[10] == 0xff && o[11] == 0xff;
    let compatible = o[..12].iter().all(|byte| *byte == 0);
    let nat64 = o[0] == 0
        && o[1] == 0x64
        && o[2] == 0xff
        && o[3] == 0x9b
        && o[4..12].iter().all(|byte| *byte == 0);
    (mapped || compatible || nat64) && refuses_v4(Ipv4Addr::new(o[12], o[13], o[14], o[15]))
}

/// Judges every address resolution returns for an accepted host — the
/// resolve-then-deny seam the resolver side (the later download slice) must
/// run per returned IP. Pure: no network I/O here.
pub fn refuses_address(address: IpAddr) -> Result<(), FetchRefusal> {
    let refused = match address {
        IpAddr::V4(v4) => refuses_v4(v4),
        IpAddr::V6(v6) => refuses_v6(v6),
    };
    if refused {
        Err(FetchRefusal::DisallowedAddress {
            host: address.to_string(),
        })
    } else {
        Ok(())
    }
}

/// `localhost` and every subdomain of it are loopback by name (RFC 6761);
/// refused before the allowlist could ever declare them.
fn is_loopback_name(domain: &str) -> bool {
    let name = domain.strip_suffix('.').unwrap_or(domain);
    name.eq_ignore_ascii_case("localhost") || name.to_ascii_lowercase().ends_with(".localhost")
}
