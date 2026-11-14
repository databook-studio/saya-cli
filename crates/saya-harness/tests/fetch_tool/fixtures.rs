use super::*;

// --- the fixture ------------------------------------------------------------

pub(super) const HOST: &str = "corpus.example.org";

/// Declared destinations, served by the local site; the DNS table decides
/// what each name resolves to on this hermetic network. `unresolvable.invalid`
/// is declared so the no-DNS-entry case reaches the resolver and fails there,
/// rather than dying earlier at the undeclared-destination gate.
pub(super) fn destinations() -> Vec<FetchDestination> {
    vec![
        FetchDestination::new("https", HOST),
        FetchDestination::new("https", "private-resolve.example.org"),
        FetchDestination::new("https", "public-resolve.example.org"),
        FetchDestination::new("https", "unresolvable.invalid"),
    ]
}

pub(super) fn dns_table() -> HashMap<String, Vec<IpAddr>> {
    HashMap::from([
        (
            HOST.to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))],
        ),
        (
            "public-resolve.example.org".to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9))],
        ),
        (
            "private-resolve.example.org".to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))],
        ),
    ])
}

/// Builds a site with the given routes and a tool whose policy declares
/// [`destinations`]. Returns the body-pull counter with them for assertions.
pub(super) async fn fixture(
    routes: Vec<(&'static str, CannedResponse)>,
    limits: FetchLimits,
) -> (LocalSite, HttpFetchTool, Arc<AtomicUsize>) {
    let site = LocalSite::spawn(routes).await;
    let pulls = Arc::new(AtomicUsize::new(0));
    let transport = TestNet {
        site: site.addr,
        dns: dns_table(),
        body_pulls: pulls.clone(),
    };
    let tool = HttpFetchTool::new(
        FetchPolicy::new(destinations()),
        Arc::new(transport),
        limits,
    );
    (site, tool, pulls)
}
