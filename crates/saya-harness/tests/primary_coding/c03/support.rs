use std::fs;

use saya_harness::coding::{
    policy::{
        ChildAuthorityRequest, ChildRole, CommandCheckoutProfile, PlatformSupport,
        RootDelegationInput,
    },
    worktree::OwnershipInput,
};
use saya_harness::runner::{SharedCredentialSource, StaticCredentialSource};
use saya_types::{
    AgentId, Capabilities, InvocationId, RepositoryId, RunnerScope, Sha256Digest, SnapshotId,
};
use std::sync::Arc;

#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "../w08/support.rs"]
mod active;
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "../w05/create_support.rs"]
mod create_support;
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "../w02_support.rs"]
mod w02_shared;
// The shared create fixture imports its git helpers as `super::support::`;
// keep that path resolving for this copy under the renamed module.
use w02_shared as support;

pub(super) struct Fixture {
    active: active::Active,
    authority: saya_harness::coding::policy::ChildAuthorityView,
    pub(super) commands: &'static super::c02_support::Fixture,
    pub(super) output_root: std::path::PathBuf,
}

pub(super) async fn fixture(label: &str) -> Option<Fixture> {
    fixture_with_program(label, super::c02_support::HELPER).await
}

pub(super) const DIAGNOSTIC_HELPER: &str = "saya-c04-helper";

pub(super) async fn diagnostic_fixture(label: &str) -> Fixture {
    let commands = super::c02_support::fixture()
        .expect("diagnostic acceptance requires proven host confinement");
    static COMPILED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    COMPILED.get_or_init(|| {
        let source = commands.program_dir().join("c04-helper.rs");
        fs::write(&source, include_str!("../c04_helper.rs")).unwrap();
        let status =
            std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .args(["--edition=2021", "-o"])
                .arg(commands.program_dir().join(DIAGNOSTIC_HELPER))
                .arg(source)
                .status()
                .unwrap();
        assert!(status.success(), "native diagnostic helper compiles");
    });
    fixture_with_program(label, DIAGNOSTIC_HELPER)
        .await
        .unwrap()
}

async fn fixture_with_program(label: &str, program: &str) -> Option<Fixture> {
    let commands = super::c02_support::fixture()?;
    let active = active::active(label, "checkout-c03-real-exit").await;
    let checkout = active.verified.checkout_id().clone();
    let runner = RunnerScope::new(vec![program.to_owned()]).unwrap();
    let mut capabilities = Capabilities::default();
    capabilities.runner = Some(runner);
    let root = approval_support::approved_root(RootDelegationInput {
        job_id: active.fixture.job.clone(),
        agent_id: AgentId::parse("agent-c03").unwrap(),
        invocation_id: InvocationId::parse("invocation-c03").unwrap(),
        repository_id: active.fixture.captured.repository_id().clone(),
        snapshot_id: active.verified.snapshot_id().clone(),
        managed_checkout_id: Some(checkout),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 0,
        write_paths: Vec::new(),
        capabilities: capabilities.clone(),
        database_profiles: Vec::new(),
    })
    .unwrap();
    let authority = root
        .narrow(
            ChildAuthorityRequest::new(
                ChildRole::Implementer,
                Vec::new(),
                capabilities,
                Vec::new(),
            )
            .unwrap(),
            PlatformSupport {
                command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
                ..PlatformSupport::default()
            },
        )
        .unwrap();
    let output_root = active.fixture.temp.path().join("c03-output");
    super::c02_support::create_private_dir(&output_root);
    let output_root = fs::canonicalize(output_root).unwrap();
    Some(Fixture {
        active,
        authority,
        commands,
        output_root,
    })
}

impl Fixture {
    pub(super) fn ledger(&self) -> &saya_harness::coding::budget::JobBudgetLedger {
        &self.active.fixture.ledger
    }

    pub(super) fn ownership(&self) -> OwnershipInput<'_> {
        OwnershipInput {
            store: &self.active.fixture.store,
            service: &self.active.fixture.service,
            repository: &self.active.fixture.repository,
            artifacts: &self.active.fixture.artifacts,
            job_id: &self.active.fixture.job,
            checkout_id: self.active.verified.checkout_id(),
        }
    }

    pub(super) fn authority(&self) -> &saya_harness::coding::policy::ChildAuthorityView {
        &self.authority
    }

    pub(super) fn unrelated_authority(
        &self,
        snapshot: bool,
    ) -> saya_harness::coding::policy::ChildAuthorityView {
        self.authority_with_identity(
            if snapshot {
                self.authority.repository_id().clone()
            } else {
                RepositoryId::parse("unrelated-repository").unwrap()
            },
            if snapshot {
                SnapshotId::parse("unrelated-snapshot").unwrap()
            } else {
                self.authority.snapshot_id().clone()
            },
            self.authority.policy_digest().clone(),
            self.authority.revocation_epoch(),
        )
    }

    pub(super) fn authority_with_identity(
        &self,
        repository_id: RepositoryId,
        snapshot_id: SnapshotId,
        policy_digest: Sha256Digest,
        revocation_epoch: u64,
    ) -> saya_harness::coding::policy::ChildAuthorityView {
        let view = &self.authority;
        let root = approval_support::approved_root(RootDelegationInput {
            job_id: view.job_id().clone(),
            agent_id: view.agent_id().clone(),
            invocation_id: view.invocation_id().clone(),
            repository_id,
            snapshot_id,
            managed_checkout_id: view.managed_checkout_id().cloned(),
            policy_digest,
            revocation_epoch,
            write_paths: Vec::new(),
            capabilities: view.capabilities().clone(),
            database_profiles: Vec::new(),
        })
        .unwrap();
        root.narrow(
            ChildAuthorityRequest::new(
                ChildRole::Implementer,
                Vec::new(),
                view.capabilities().clone(),
                Vec::new(),
            )
            .unwrap(),
            PlatformSupport {
                command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
                ..PlatformSupport::default()
            },
        )
        .unwrap()
    }

    pub(super) fn commit_original_candidate_source(&self) {
        fs::write(
            self.active.root.join("tracked.txt"),
            b"modified tracked bytes\n",
        )
        .unwrap();
        self.active
            .fixture
            .git_output(&self.active.root, &["add", "-A"]);
        self.active.fixture.git_output(
            &self.active.root,
            &[
                "-c",
                "user.name=saya",
                "-c",
                "user.email=saya@example.com",
                "commit",
                "-m",
                "move candidate HEAD",
            ],
        );
    }

    pub(super) fn change_candidate_source(&self) {
        fs::write(
            self.active.root.join("tracked.txt"),
            b"changed after check\n",
        )
        .unwrap();
    }

    pub(super) fn set_candidate_source(&self, bytes: &[u8]) {
        fs::write(self.active.root.join("tracked.txt"), bytes).unwrap();
    }

    pub(super) fn resolver(&self) -> SharedCredentialSource {
        Arc::new(StaticCredentialSource::new(Vec::<(String, String)>::new()))
    }
}

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[path = "../approval_support.rs"]
mod approval_support;
