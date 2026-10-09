//! The cancellation battery for `http_download`: the turn's token rides the
//! download from admission through every wait to the point right before
//! publication, and a cancelled turn never publishes an incomplete file.
//!
//! Served entirely from in-memory fakes — no test touches the real network.
//! The cases: a pre-cancelled download writes nothing; a cancelled header
//! wait writes nothing; a cancelled mid-body leaves an exact, resumable
//! partial (the sidecar records exactly the served bytes) and a retry
//! completes to the whole-file digest; a cancellation after the last byte
//! but before publication publishes nothing; the tool adapter reports the
//! cancellation typed; and the old uncancelled wrapper still completes.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{CancellationToken, ToolError, ToolExecutionContext, ToolExecutor};
use saya_harness::fetch::{
    DownloadBudget, DownloadCall, DownloadError, DownloadLimits, DownloadOutcome, FetchBody,
    FetchDestination, FetchLimits, FetchPolicy, FetchRequest, FetchTools, FetchTransport,
    FetchTransportError, WireResponse, http_download, http_download_cancellable,
};
use saya_harness::workspace::Workspace;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Every await is under this guard, so a regression hangs into a failure
/// instead of stalling the suite.
const ABORT: Duration = Duration::from_secs(10);

// --- the fixture ------------------------------------------------------------

const HOST: &str = "files.example.org";
const DEST: &str = "downloads/doc.bin";
const OK_DEST: &str = "downloads/ok.bin";
const PART: &str = "downloads/doc.bin.saya-part";
const SIDECAR: &str = "downloads/doc.bin.saya-part.json";

/// The public TEST-NET address every fake resolves the declared host to —
/// allowed by the address refusal, like the real battery's table.
const RESOLVED: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));

fn policy() -> FetchPolicy {
    FetchPolicy::new(vec![FetchDestination::new("https", HOST)])
}

fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

/// One isolated workspace under the temp dir.
fn workspace(label: &str) -> (PathBuf, Workspace) {
    let root = std::env::temp_dir().join(format!(
        "saya-harness-download-cancel-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp workspace");
    let workspace = Workspace::open(&root).expect("workspace opens");
    (root, workspace)
}

/// The lowercase-hex SHA-256 of `content`.
fn digest(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sidecar_json(root: &Path) -> serde_json::Value {
    let bytes = std::fs::read(root.join(SIDECAR)).expect("the sidecar exists");
    serde_json::from_slice(&bytes).expect("the sidecar parses")
}

/// One download under the turn's token, spawned so the test can cancel while
/// it runs. The wallet moves in by clone, so the caller's clone sees the
/// claims.
fn spawn_download(
    net: impl FetchTransport + 'static,
    workspace: Workspace,
    budget: DownloadBudget,
    token: CancellationToken,
) -> JoinHandle<Result<DownloadOutcome, DownloadError>> {
    tokio::spawn(async move {
        let policy = policy();
        http_download_cancellable(DownloadCall {
            policy: &policy,
            transport: &net,
            workspace: &workspace,
            destination: DEST,
            expected_destination_sha256: None,
            url: &url("/doc"),
            limits: DownloadLimits::default(),
            budget: &budget,
            cancellation: &token,
        })
        .await
    })
}

// --- the fakes: one machine, canned bodies ----------------------------------

/// Logs every GET's URL and returns a canned body.
fn logged(url: &str, calls: &Arc<Mutex<Vec<String>>>) {
    calls.lock().expect("calls").push(url.to_owned());
}

// --- test a: pre-cancelled ---------------------------------------------------

// --- test b: cancelled header wait -------------------------------------------

// --- test c: cancelled mid-body, exact partial, retry completes --------------

// --- test d: cancelled after the last byte, before publication ---------------

// --- test e: the tool adapter reports cancellation typed ----------------------

// --- test f: the old wrapper still completes ----------------------------------

#[path = "download_cancel/transports.rs"]
mod transports;

#[path = "download_cancel/admission.rs"]
mod admission;

#[path = "download_cancel/resumption.rs"]
mod resumption;

#[path = "download_cancel/publication.rs"]
mod publication;

#[path = "download_cancel/adapters.rs"]
mod adapters;
