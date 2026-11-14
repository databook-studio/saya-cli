use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest as _, Sha512};

use crate::{
    fetch::{FetchBody, FetchRequest, FetchTransport, FetchTransportError, WireResponse},
    runtime::{
        NpmCheckAction, NpmCheckRequest, NpmCheckSource, NpmInstallConsent, NpmRuntimeError,
        install_npm_inputs, run_npm_check,
    },
};

use super::{integrity, package_lock, package_manifest, private_root, project, write_project};

fn project_for_tarball(
    tarball: &[u8],
    sri_fill: Option<u8>,
) -> (std::path::PathBuf, crate::workspace::Workspace) {
    let root = private_root("npm-effect-project");
    let integrity = match sri_fill {
        Some(fill) => integrity(fill),
        None => format!("sha512-{}", STANDARD.encode(Sha512::digest(tarball))),
    };
    write_project(
        &root,
        package_manifest("react-dashboard", "1.0.0"),
        package_lock(
            "react-dashboard",
            "1.0.0",
            3,
            "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
            &integrity,
        ),
    );
    let project = crate::workspace::Workspace::open(&root).expect("test project opens");
    (root, project)
}

#[derive(Clone, Default)]
struct CountingTransport {
    calls: Arc<AtomicUsize>,
    resolves: Arc<AtomicUsize>,
    gets: Arc<AtomicUsize>,
    body: Arc<Vec<u8>>,
}

impl CountingTransport {
    fn with_body(body: &[u8]) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            resolves: Arc::new(AtomicUsize::new(0)),
            gets: Arc::new(AtomicUsize::new(0)),
            body: Arc::new(body.to_vec()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn resolutions(&self) -> usize {
        self.resolves.load(Ordering::Relaxed)
    }

    fn gets(&self) -> usize {
        self.gets.load(Ordering::Relaxed)
    }
}

struct StaticBody(Option<Vec<u8>>);

#[async_trait]
impl FetchBody for StaticBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.0.take())
    }
}

#[async_trait]
impl FetchTransport for CountingTransport {
    async fn resolve(&self, _host: &str) -> Result<Vec<std::net::IpAddr>, FetchTransportError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.resolves.fetch_add(1, Ordering::Relaxed);
        Ok(vec![std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            93, 184, 216, 34,
        ))])
    }

    async fn get(&self, _request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.gets.fetch_add(1, Ordering::Relaxed);
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            content_type: None,
            body: Box::new(StaticBody(Some(self.body.as_ref().clone()))),
        })
    }
}

#[tokio::test]
async fn npm_input_install_refuses_missing_runtime_before_fetching() {
    let (root, workspace) = project(
        "react-dashboard",
        "1.0.0",
        3,
        "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
        &integrity(21),
    );
    let state_root = root.join("state");
    let transport = CountingTransport::default();

    let result = install_npm_inputs(
        &state_root,
        &workspace,
        Some(NpmInstallConsent::Granted),
        &transport,
        &saya_agent::CancellationToken::new(),
    )
    .await;

    assert_eq!(result, Err(NpmRuntimeError::NodeRuntimeNotReady));
    assert_eq!(transport.calls(), 0);
    drop(workspace);
    std::fs::remove_dir_all(root).unwrap();
}

#[path = "npm_effect_tests.rs"]
mod effects;

#[path = "npm_process_tests.rs"]
mod process;
#[path = "npm_state_tests.rs"]
mod state;

#[tokio::test]
async fn native_check_refuses_missing_runtime_before_check_execution() {
    let (root, workspace) = project(
        "react-dashboard",
        "1.0.0",
        3,
        "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
        &integrity(22),
    );
    let state_root = root.join("state");
    let cancellation = saya_agent::CancellationToken::new();
    let source_sha256 = "a".repeat(64);

    let result = run_npm_check(NpmCheckRequest {
        state_root: &state_root,
        project: &workspace,
        source_sha256: &source_sha256,
        source: NpmCheckSource::OperatorProject,
        action: NpmCheckAction::TestCi,
        timeout: std::time::Duration::from_secs(30),
        cancellation: &cancellation,
    })
    .await;

    assert_eq!(result, Err(NpmRuntimeError::NodeRuntimeNotReady));
    drop(workspace);
    std::fs::remove_dir_all(root).unwrap();
}
