use super::transport::TestNet;
use super::*;

// --- the fixture ------------------------------------------------------------

pub(super) const HOST: &str = "files.example.org";
pub(crate) const DEST: &str = "downloads/doc.bin";
pub(crate) const PART: &str = "downloads/doc.bin.saya-part";
pub(crate) const SIDECAR: &str = "downloads/doc.bin.saya-part.json";

fn destinations() -> Vec<FetchDestination> {
    vec![
        FetchDestination::new("https", HOST),
        FetchDestination::new("https", "private-resolve.example.org"),
        FetchDestination::new("https", "public-resolve.example.org"),
    ]
}

fn dns_table() -> HashMap<String, Vec<IpAddr>> {
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

pub(crate) fn policy() -> FetchPolicy {
    FetchPolicy::new(destinations())
}

pub(crate) fn transport(site: &LocalSite) -> TestNet {
    TestNet {
        site: site.addr,
        dns: dns_table(),
    }
}

pub(crate) fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

/// One isolated workspace under the temp dir.
pub(crate) fn workspace(label: &str) -> (PathBuf, Workspace) {
    let root = std::env::temp_dir().join(format!(
        "saya-harness-download-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp workspace");
    let workspace = Workspace::open(&root).expect("workspace opens");
    (root, workspace)
}

/// The lowercase-hex SHA-256 of `content` — the digest the download must
/// match.
pub(crate) fn digest(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn sidecar_json(root: &std::path::Path) -> serde_json::Value {
    let bytes = std::fs::read(root.join(SIDECAR)).expect("the sidecar exists");
    serde_json::from_slice(&bytes).expect("the sidecar parses")
}
