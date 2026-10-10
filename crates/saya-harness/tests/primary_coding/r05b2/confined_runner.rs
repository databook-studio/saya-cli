//! R05b2a confined-runner battery (authored, not executed): the proven
//! confinement's `PlatformSupport` bridge and the confined run_program tool
//! the confinement hands out in already-confined form.

use std::{path::Path, sync::Arc, time::Duration};

use saya_agent::{CancellationToken, ToolExecutor};
use saya_harness::coding::policy::{CommandCheckoutProfile, PlatformSupport};
use saya_harness::coding::worktree::{
    OwnershipInput, confine_checkout, create_managed_checkout, verify_owned_checkout,
};
use saya_harness::runner::{RunProgram, SharedCredentialSource, StaticCredentialSource};
use saya_types::{JobId, ManagedCheckoutId, RunnerScope};

use super::create_support::CreateCheckoutFixture;
use super::support;

/// The checkout under confinement in every test.
const A_ID: &str = "checkout-r05b2-a";

/// One ownership input over the fixture for the given asking job.
fn ownership_input<'a>(
    fixture: &'a CreateCheckoutFixture,
    job_id: &'a JobId,
    checkout_id: &'a ManagedCheckoutId,
) -> OwnershipInput<'a> {
    OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id,
        checkout_id,
    }
}

/// The runner tool's credential resolver: a static source with no secrets.
fn resolver() -> SharedCredentialSource {
    Arc::new(StaticCredentialSource::new([]))
}

/// The fixture-to-proof flow every test walks: create, verify, confine to /bin.
macro_rules! confined {
    ($fixture:ident, $a:ident, $token:ident) => {{
        create_managed_checkout(&$fixture.input(&$a), &$token)
            .await
            .unwrap();
        let input = ownership_input(&$fixture, &$fixture.job, &$a);
        let verified = verify_owned_checkout(&input, &$token).await.unwrap();
        confine_checkout(&verified, Path::new("/bin")).unwrap()
    }};
}

/// A proven confinement is itself the evidence: it maps to the trusted host
/// support — edits in exactly the owned checkout, commands under an isolated
/// private checkout, no network, no database reads.
#[tokio::test]
async fn proven_confinement_maps_to_isolated_private_checkout() {
    let fixture = CreateCheckoutFixture::new("r05b2-bridge").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    let confined = confined!(fixture, a, token);

    let platform: PlatformSupport = confined.platform_support();
    assert!(
        platform.workspace_edits,
        "the single writable root is the owned checkout"
    );
    assert!(matches!(
        platform.command_profile,
        Some(CommandCheckoutProfile::IsolatedPrivateCheckout)
    ));
    assert!(!platform.network_fetch, "the net is empty by construction");
    assert!(!platform.database_reads);
}

/// The runner tool over the confinement runs a scoped program with the owned
/// checkout as its working directory.
#[tokio::test]
async fn runner_tool_runs_a_scoped_program_in_the_checkout() {
    let fixture = CreateCheckoutFixture::new("r05b2-runner").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    let confined = confined!(fixture, a, token);

    let records = support::TempRoot::new("r05b2-records");
    let scope = RunnerScope::new(vec!["pwd".to_string()]).unwrap();
    let tool: RunProgram = confined.runner_tool(
        Some(scope),
        None,
        Duration::from_secs(10),
        resolver(),
        records.path().to_path_buf(),
    );
    let outcome = tool.run("pwd", &[], None).await.unwrap();
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "pwd failed: {:?}",
        outcome.stderr.text
    );
    let checkout =
        std::fs::canonicalize(fixture.service.managed_path(&a).unwrap().as_str()).unwrap();
    assert_eq!(
        outcome.stdout.text.trim(),
        checkout.to_string_lossy(),
        "the child's cwd is the owned checkout"
    );
}

/// A program outside the tool's scope is refused before any process runs.
#[tokio::test]
async fn runner_tool_refuses_a_program_outside_its_scope() {
    let fixture = CreateCheckoutFixture::new("r05b2-refuse").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    let confined = confined!(fixture, a, token);

    let records = support::TempRoot::new("r05b2-records");
    let scope = RunnerScope::new(vec!["pwd".to_string()]).unwrap();
    let tool = confined.runner_tool(
        Some(scope),
        None,
        Duration::from_secs(10),
        resolver(),
        records.path().to_path_buf(),
    );
    let refused = tool.run("ls", &[], None).await;
    assert!(
        refused.is_err(),
        "a program outside the scope must be refused before any spawn"
    );
}

/// Without a runner scope the tool has no door at all.
#[tokio::test]
async fn runner_tool_without_a_runner_scope_has_no_door() {
    let fixture = CreateCheckoutFixture::new("r05b2-no-scope").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    let confined = confined!(fixture, a, token);

    let records = support::TempRoot::new("r05b2-records");
    let tool = confined.runner_tool(
        None,
        None,
        Duration::from_secs(10),
        resolver(),
        records.path().to_path_buf(),
    );
    let refused = tool.run("pwd", &[], None).await;
    assert!(refused.is_err());
}

/// The host-side outcome record goes to the trusted record directory the
/// composition root hands the tool — never inside the confined checkout,
/// where a child-controlled symlink could redirect the host's writes.
#[tokio::test]
async fn runner_records_never_land_inside_the_checkout() {
    let fixture = CreateCheckoutFixture::new("r05b2-records").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    let confined = confined!(fixture, a, token);
    let records = support::TempRoot::new("r05b2-records");

    let scope = RunnerScope::new(vec!["pwd".to_string()]).unwrap();
    let tool = confined.runner_tool(
        Some(scope),
        None,
        Duration::from_secs(10),
        resolver(),
        records.path().to_path_buf(),
    );
    tool.run("pwd", &[], None).await.unwrap();
    let checkout = fixture.service.managed_path(&a).unwrap();
    let stray = Path::new(checkout.as_str()).join("run_program");
    assert!(!stray.exists(), "the checkout gained {stray:?}");

    let executed = tool
        .execute(
            "run_program",
            serde_json::json!({"program": "pwd", "args": [], "timeout_seconds": 10}),
        )
        .await
        .unwrap();
    assert!(
        executed.get("record_error").is_none(),
        "the record failed: {executed}"
    );
    let recorded = std::fs::read_dir(records.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        recorded.iter().any(|name| name.ends_with(".json")),
        "no outcome record landed in the trusted dir: {recorded:?}"
    );
    assert!(!stray.exists(), "the checkout gained {stray:?}");
}
