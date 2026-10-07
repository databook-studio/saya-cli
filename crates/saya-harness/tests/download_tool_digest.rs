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

/// A no-digest call over an existing destination is create-only: it refuses
/// before the transport is ever asked, and the file is untouched.
#[tokio::test]
async fn a_call_without_a_digest_over_an_existing_destination_refuses_before_the_transport() {
    let on_wire = b"the body the call wanted".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("create-only");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget.clone());
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({"url": url("/doc"), "destination": DEST}),
        )
        .await
        .expect_err("create-only refuses an existing destination");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("already exists"),
        "the create-only refusal names the collision: {detail}"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
    assert!(
        calls.lock().expect("calls").is_empty(),
        "the refusal happens before any transport call"
    );
    assert_eq!(budget.consumed(), 0, "no budget was spent");
}

/// A malformed digest — not exactly 64 lowercase hex characters — refuses
/// at the argument boundary, before any transport call or budget use, with
/// an error that names the shape but never reveals or computes a digest.
#[tokio::test]
async fn a_malformed_digest_refuses_before_any_transport_call_or_budget_use() {
    let on_wire = b"never fetched".to_vec();
    let (_root, workspace) = workspace("malformed");
    for bad in ["ZZ", &"a".repeat(63), &"A".repeat(64), &"g".repeat(64)] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let budget = DownloadBudget::default();
        let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
        let tools = tools(net, workspace.clone(), budget.clone());
        let error = tools
            .execute(
                "http_download",
                serde_json::json!({
                    "url": url("/doc"),
                    "destination": DEST,
                    "expected_digest": bad,
                }),
            )
            .await
            .expect_err("a malformed digest is refused");
        let ToolError::Fetch(detail) = error else {
            panic!("the lane's typed error, got: {error:?}")
        };
        assert!(
            detail.contains("expected_digest"),
            "the refusal names the argument: {detail}"
        );
        assert!(
            !detail.contains(&digest(&on_wire)),
            "the refusal never computes a digest: {detail}"
        );
        assert!(
            calls.lock().expect("calls").is_empty(),
            "the malformed digest never reaches the transport"
        );
        assert_eq!(budget.consumed(), 0, "no budget was spent");
    }
    assert!(
        !workspace.root().join(DEST).exists(),
        "nothing landed anywhere"
    );
}

/// A non-string digest is its own typed argument error, like the guarded
/// file tools', and never reaches the transport.
#[tokio::test]
async fn a_non_string_digest_is_a_typed_argument_error() {
    let on_wire = b"never fetched".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("non-string");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": 7,
            }),
        )
        .await
        .expect_err("a non-string digest is a typed argument error");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("expected_digest must be a string"),
        "the typed argument error: {detail}"
    );
    assert!(
        calls.lock().expect("calls").is_empty(),
        "the argument error never reaches the transport"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
}

/// An explicit null digest is absent — the same convention the guarded file
/// tools' argument parsers follow — so over an existing destination the
/// create-only refusal applies.
#[tokio::test]
async fn an_explicit_null_digest_is_the_create_form_over_an_existing_destination() {
    let on_wire = b"never fetched".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("null-digest");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": serde_json::Value::Null,
            }),
        )
        .await
        .expect_err("null is absent, so create-only refuses the existing destination");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("already exists"),
        "the create-only refusal: {detail}"
    );
    assert!(
        calls.lock().expect("calls").is_empty(),
        "the create-only refusal happens before the transport"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
}

/// A matching digest replaces the destination with the streamed body, and
/// the result metadata keeps its shape: destination, bytes, sha256.
#[tokio::test]
async fn a_matching_digest_replaces_the_destination_and_the_result_shape_is_unchanged() {
    let on_wire = b"the replacement body".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("matching");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let value = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": digest(&existing),
            }),
        )
        .await
        .expect("the observed digest authorizes the replacement");
    assert_eq!(
        value["destination"].as_str(),
        Some(DEST),
        "the result names the destination"
    );
    assert_eq!(
        value["bytes"].as_u64(),
        Some(on_wire.len() as u64),
        "the result counts the streamed bytes"
    );
    assert_eq!(
        value["sha256"].as_str(),
        Some(digest(&on_wire).as_str()),
        "the result carries the new digest"
    );
    assert_eq!(on_disk(&workspace), on_wire, "the body replaced the file");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "exactly the one request was served"
    );
}

/// A stale digest — not the destination's current content — refuses
/// `ContentChanged` at the guarded rename and leaves the file byte-for-byte
/// unchanged; the precondition is judged against the destination, so the
/// transport was asked and refused to publish.
#[tokio::test]
async fn a_stale_digest_refuses_and_leaves_the_destination_unchanged() {
    let on_wire = b"the body the call wanted".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("stale");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": digest(&on_wire),
            }),
        )
        .await
        .expect_err("the digest names other content than the destination holds");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("does not match the expected_digest"),
        "the guarded stale-digest refusal: {detail}"
    );
    assert!(
        !detail.contains(&digest(&on_wire)),
        "the refusal never echoes the supplied digest: {detail}"
    );
    assert!(
        !detail.contains(&digest(&existing)),
        "the refusal never leaks the destination's real digest: {detail}"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "the precondition is judged at the rename, after the one request"
    );
}

/// An invented digest — 64 zeros, observed from no read of anything — is
/// refused at the guarded rename, and the refusal carries no 64-hex
/// substring at all: nothing a model can mine to learn the destination's
/// real digest and replace a file it never read.
#[tokio::test]
async fn an_invented_digest_cannot_learn_the_real_one() {
    let on_wire = b"the body the call wanted".to_vec();
    let existing = b"what is already there".to_vec();
    let (_root, workspace) = workspace("invented");
    seed_destination(&workspace, &existing);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": "0".repeat(64),
            }),
        )
        .await
        .expect_err("an invented digest is refused");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        !contains_64_hex(&detail),
        "the refusal never carries a digest-shaped substring: {detail}"
    );
    assert_eq!(on_disk(&workspace), existing, "the file is untouched");
}

/// A digest over a missing destination is a refusal, never creation: the
/// precondition can only be judged against bytes that exist.
#[tokio::test]
async fn a_digest_over_a_missing_destination_refuses_and_creates_nothing() {
    let on_wire = b"the body the call wanted".to_vec();
    let (_root, workspace) = workspace("missing");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let budget = DownloadBudget::default();
    let net = LoggedNet::serve(&on_wire, 8, Arc::clone(&calls));
    let tools = tools(net, workspace.clone(), budget);
    let error = tools
        .execute(
            "http_download",
            serde_json::json!({
                "url": url("/doc"),
                "destination": DEST,
                "expected_digest": digest(&on_wire),
            }),
        )
        .await
        .expect_err("a digest never authorizes creation");
    let ToolError::Fetch(detail) = error else {
        panic!("the lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains("does not exist"),
        "the missing-destination refusal: {detail}"
    );
    assert!(!workspace.root().join(DEST).exists(), "nothing was created");
}

/// The run lane's definition advertises the optional argument with the same
/// name and shape the guarded file tools use, and the required list is
/// unchanged.
#[test]
fn the_run_lane_definition_advertises_the_optional_digest() {
    let definition = http_download_definition();
    let properties = definition.parameters["properties"]
        .as_object()
        .expect("the schema has properties");
    let declared = properties
        .get("expected_digest")
        .expect("expected_digest is declared");
    assert_eq!(declared["type"], "string", "the digest is a string");
    let description = declared["description"].as_str().expect("described");
    assert!(
        description.contains("never invent or guess"),
        "the description forbids invented digests: {description}"
    );
    assert!(
        description.contains("read it first"),
        "the description requires observing the digest from a read: {description}"
    );
    assert_eq!(
        definition.parameters["required"],
        serde_json::json!(["url", "destination"]),
        "the required list is unchanged: the digest stays optional"
    );
    assert_eq!(
        definition.parameters["additionalProperties"],
        serde_json::json!(false),
        "the schema stays closed"
    );
}
