//! The `http_download` tool lane's destination-digest argument: the adapter
//! forwards the caller's precondition instead of stripping it, and the
//! argument is judged — type and shape — before any transport call or
//! budget use. The guarded outcome itself (create-only, digest-replace at
//! the rename) is proven at the call level in `download.rs`; these are the
//! tool-lane cases the new argument rides.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchBody, FetchDestination, FetchLimits, FetchPolicy,
    FetchRequest, FetchTools, FetchTransport, FetchTransportError, WireResponse,
    http_download_definition,
};
use saya_harness::workspace::Workspace;

const HOST: &str = "files.example.org";
const DEST: &str = "downloads/doc.bin";

/// The public TEST-NET address every fake resolves the declared host to.
const RESOLVED: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));

fn policy() -> FetchPolicy {
    FetchPolicy::new(vec![FetchDestination::new("https", HOST)])
}

fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

/// One isolated workspace under the temp dir.
fn workspace(label: &str) -> (PathBuf, Arc<Workspace>) {
    let root = std::env::temp_dir().join(format!(
        "saya-harness-download-tool-digest-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp workspace");
    let workspace = Workspace::open(&root).expect("workspace opens");
    (root, Arc::new(workspace))
}

/// The lowercase-hex SHA-256 of `content`.
fn digest(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `text` carries any run of 64 hex characters — a digest-shaped
/// substring, however it arrived.
fn contains_64_hex(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 64
        && (0..=bytes.len() - 64).any(|start| {
            bytes[start..start + 64]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit())
        })
}

/// The step's adapter over a logged, canned transport: one 200 body, chunk
/// by chunk, every GET's URL recorded.
fn tools(net: LoggedNet, workspace: Arc<Workspace>, budget: DownloadBudget) -> FetchTools {
    FetchTools::new(
        policy(),
        Arc::new(net),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(workspace),
        budget,
    )
}

struct LoggedNet {
    chunks: Vec<Vec<u8>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl LoggedNet {
    fn serve(body: &[u8], chunk: usize, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            chunks: if chunk >= body.len() {
                vec![body.to_vec()]
            } else {
                body.chunks(chunk).map(<[u8]>::to_vec).collect()
            },
            calls,
        }
    }
}

#[async_trait]
impl FetchTransport for LoggedNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        self.calls
            .lock()
            .expect("calls")
            .push(request.url.as_str().to_owned());
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(CannedBody {
                chunks: VecDeque::from(self.chunks.clone()),
            }),
        })
    }
}

struct CannedBody {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl FetchBody for CannedBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.chunks.pop_front())
    }
}

/// Seeds `DEST` with `content` and returns the workspace handle.
fn seed_destination(workspace: &Workspace, content: &[u8]) {
    let path = workspace.root().join(DEST);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("seed dir");
    std::fs::write(path, content).expect("seed destination");
}

fn on_disk(workspace: &Workspace) -> Vec<u8> {
    std::fs::read(workspace.root().join(DEST)).expect("the destination exists")
}

#[path = "download_tool_digest/digest_arguments.rs"]
mod digest_arguments;

#[path = "download_tool_digest/destination_replacement.rs"]
mod destination_replacement;
