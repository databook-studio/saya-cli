//! The direct-mode root's one managed working checkout: an agent-less
//! checkout at the job's base commit, deterministically named over the job id
//! and reused on resume through a bounded private record.

use super::{CodingError, NativeCodingJob, context, create::digest, verify_root_checkout};
use saya_harness::coding::git::IgnoredPaths;
use saya_harness::coding::integration::{DEFAULT_DELTA_DEADLINE, DeltaKind, compute_delta};
use saya_harness::coding::local::LocalBaseline;
use saya_harness::coding::worktree::{InPlaceRootInput, VerifiedCheckout, verify_in_place_root};
use saya_harness::workspace::Workspace;
use saya_types::{ManagedCheckoutId, RepositoryId, Sha256Digest, SnapshotId};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// The bound for the private record naming the root checkout, and the one
/// naming an in-place root.
const MAX_ROOT_CHECKOUT_RECORD_BYTES: usize = 1024;

/// The private record naming one in-place root, the fixed baseline it was
/// captured from, and what its start found: whether the folder is a Git
/// working tree and the HEAD it held. It is identity and display only — every
/// filesystem action still goes through the contained workspace and the
/// baseline's own digest.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InPlaceRootRecord {
    version: u32,
    root_id: RepositoryId,
    root_path: String,
    snapshot_id: SnapshotId,
    plan_sha256: Sha256Digest,
    #[serde(default)]
    git: bool,
    #[serde(default)]
    start_head: Option<String>,
}

/// The record shape this build writes. A v1 record came from a development
/// build whose whole-folder baseline no longer exists.
const IN_PLACE_RECORD_VERSION: u32 = 2;

impl InPlaceRootRecord {
    pub(crate) fn new(baseline: &LocalBaseline, git: bool, start_head: Option<String>) -> Self {
        Self {
            version: IN_PLACE_RECORD_VERSION,
            root_id: baseline.repository_id().clone(),
            root_path: baseline.root_path().to_owned(),
            snapshot_id: baseline.snapshot_id().clone(),
            plan_sha256: baseline.manifest_sha256().clone(),
            git,
            start_head,
        }
    }

    pub(crate) fn write(&self, path: &Path) -> Result<(), CodingError> {
        let bytes = serde_json::to_vec(self).map_err(CodingError::owner)?;
        context::write_new(path, &bytes, MAX_ROOT_CHECKOUT_RECORD_BYTES)
    }

    pub(crate) fn read(path: &Path) -> Result<Self, CodingError> {
        let bytes = context::read(path, MAX_ROOT_CHECKOUT_RECORD_BYTES)?;
        let record: Self = serde_json::from_slice(&bytes).map_err(CodingError::owner)?;
        if record.version != IN_PLACE_RECORD_VERSION {
            return Err(CodingError::InPlaceLegacyBaseline);
        }
        Ok(record)
    }

    /// Whether the folder was a Git working tree at start.
    pub(crate) fn git(&self) -> bool {
        self.git
    }

    pub(crate) fn root_id(&self) -> &RepositoryId {
        &self.root_id
    }

    pub(crate) fn root_path(&self) -> &str {
        &self.root_path
    }

    pub(crate) fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub(crate) fn plan_sha256(&self) -> &Sha256Digest {
        &self.plan_sha256
    }
}

/// One path the job changed against its baseline, with the kind of change: a
/// managed checkout's delta entry, or an in-place job's first touch compared
/// to its before-image. The path is the verified root's own relative path, so
/// it is display-safe as it stands.
pub(crate) struct RootChange {
    pub(crate) path: String,
    pub(crate) kind: DeltaKind,
    /// The version the path holds now; `None` for a removal.
    pub(crate) content_sha256: Option<Sha256Digest>,
    /// The bytes the path holds now; `0` for a removal.
    pub(crate) bytes: u64,
}

/// The versioned private record that names the root working checkout.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootCheckoutRecord {
    version: u32,
    checkout_id: ManagedCheckoutId,
}
impl RootCheckoutRecord {
    fn new(checkout_id: ManagedCheckoutId) -> Self {
        Self {
            version: 1,
            checkout_id,
        }
    }
    fn write(&self, path: &std::path::Path) -> Result<(), CodingError> {
        let bytes = serde_json::to_vec(self).map_err(CodingError::owner)?;
        context::write_new(path, &bytes, MAX_ROOT_CHECKOUT_RECORD_BYTES)
    }
    fn read(path: &std::path::Path) -> Result<Self, CodingError> {
        let bytes = context::read(path, MAX_ROOT_CHECKOUT_RECORD_BYTES)?;
        serde_json::from_slice(&bytes).map_err(CodingError::owner)
    }
}

impl NativeCodingJob {
    /// Ensures the direct-mode coordinator's own managed working checkout:
    /// exactly one agent-less checkout at the job's base commit, whose id is
    /// derived deterministically from the job id so a resume recomputes the
    /// same one. The private record is authoritative on resume: the recorded
    /// checkout is re-verified against the durable ownership record at use
    /// time. A job without write paths creates nothing.
    pub(crate) async fn ensure_root_checkout(
        &self,
    ) -> Result<Option<ManagedCheckoutId>, CodingError> {
        // An in-place root is the user's own folder: no managed checkout is
        // ever created for it. A read-only in-place job stays on the F4
        // read-only surface; a write job gets the deterministic in-place
        // checkout id, which names the folder and never a durable row.
        if self.application.is_in_place() {
            if self.request.write_paths.is_empty() {
                return Ok(None);
            }
            return Ok(Some(self.in_place_checkout()?.checkout_id().clone()));
        }
        if self.request.write_paths.is_empty() {
            return Ok(None);
        }
        let identity = digest(format!("{}:root", self.job_id().as_str()).as_bytes())?;
        let checkout = ManagedCheckoutId::parse(&format!("work-{}", &identity.as_str()[..32]))
            .map_err(CodingError::owner)?;
        if let Some(verified) = self.verified_root_checkout().await? {
            let checkout_id = verified.checkout_id().clone();
            // The resume verification is this start flow's proof: seed it so
            // the workspace composition and the first readiness view spend it
            // instead of walking W again.
            self.seed_root_checkout(Some(verified));
            return Ok(Some(checkout_id));
        }
        let record = self.records.join("root-checkout.json");
        let verified = self.create_checkout(&checkout).await?;
        RootCheckoutRecord::new(checkout.clone()).write(&record)?;
        // W exists now: the advisory facts computed before it did describe a
        // job without one, so they must not be reused.
        self.note_root_w_attempt();
        // The completed create's own proof, keyed to the epoch above.
        self.seed_root_checkout(verified);
        Ok(Some(checkout))
    }

    /// Whether this job's root is read-only: it approved no write path. An
    /// in-place job with a scope writes its own folder through the same
    /// workspace toolset a managed checkout uses.
    pub(crate) fn is_read_only_root(&self) -> bool {
        self.request.write_paths.is_empty()
    }

    /// Whether this job works in place over the user's own folder rather than
    /// in a managed checkout.
    pub(crate) fn is_in_place(&self) -> bool {
        self.application.is_in_place()
    }

    /// The private record naming this job's in-place root and the baseline it
    /// was fixed from.
    pub(crate) fn in_place_record(&self) -> Result<InPlaceRootRecord, CodingError> {
        InPlaceRootRecord::read(&self.records.join("inplace-root.json"))
    }

    /// The use-time proof inputs for this job's in-place root. The record is
    /// identity and display only; `verify_in_place_root` re-derives the path
    /// and the baseline from it.
    pub(crate) fn in_place_input<'a>(
        &'a self,
        record: &'a InPlaceRootRecord,
    ) -> InPlaceRootInput<'a> {
        InPlaceRootInput {
            artifacts: &self.application.artifacts,
            state_root: &self.application.state_root,
            root_id: record.root_id(),
            root_path: Path::new(record.root_path()),
            snapshot_id: record.snapshot_id(),
            plan_sha256: record.plan_sha256(),
        }
    }

    /// This job's in-place root, re-verified at use time.
    pub(crate) fn in_place_checkout(&self) -> Result<VerifiedCheckout, CodingError> {
        let record = self.in_place_record()?;
        verify_in_place_root(&self.in_place_input(&record)).map_err(CodingError::owner)
    }

    /// The job's own folder opened as a contained workspace. The read-only
    /// root reads and searches it; no checkout is created and no write is
    /// admitted.
    pub(crate) fn read_only_workspace(&self) -> Result<Arc<Workspace>, CodingError> {
        Workspace::open(self.application.root_path())
            .map(Arc::new)
            .map_err(CodingError::owner)
    }

    /// The recorded root working checkout, re-verified as the root's working
    /// checkout at use time (admitted `Active | Retained`), or `None` when no
    /// record exists yet. Read-only: unlike `ensure_root_checkout` it never
    /// creates W, so status and preparation reads stay admission-free.
    pub(crate) async fn verified_root_checkout(
        &self,
    ) -> Result<Option<VerifiedCheckout>, CodingError> {
        if self.application.is_in_place() {
            if self.request.write_paths.is_empty() {
                return Ok(None);
            }
            return self.in_place_checkout().map(Some);
        }
        let Some(checkout_id) = read_root_checkout(&self.records)? else {
            return Ok(None);
        };
        let verified = verify_root_checkout(&self.ownership(&checkout_id)?, &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        Ok(Some(verified))
    }

    /// What the root working checkout W already changed against the baseline
    /// snapshot, one entry per path in the delta's own order (sorted by path).
    /// Empty for jobs without write paths, or before the root checkout exists.
    pub(crate) async fn root_changes(&self) -> Result<Vec<RootChange>, CodingError> {
        if self.request.write_paths.is_empty() {
            return Ok(Vec::new());
        }
        let Some(verified) = self.verified_root_checkout().await? else {
            return Ok(Vec::new());
        };
        let ignored = self.checkout_ignored_paths(&verified).await?;
        super::in_place_changes::job_changes(
            &self.application.artifacts,
            &self.records,
            &verified,
            &ignored,
            &self.cancellation,
        )
    }

    /// The checkout's own Git ignore rules. An in-place root is the user's own
    /// folder and has no checkout to walk: its changes come from the
    /// before-image store, so no ignore read is spent.
    pub(crate) async fn checkout_ignored_paths(
        &self,
        verified: &VerifiedCheckout,
    ) -> Result<IgnoredPaths, CodingError> {
        if self.application.is_in_place() {
            return Ok(IgnoredPaths::none());
        }
        self.application
            .service
            .ignored_paths(
                Path::new(verified.path().as_str()),
                &self.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )
            .await
            .map_err(CodingError::owner)
    }
}

impl super::CodingApplication {
    /// The checkout's own Git ignore rules for an operator read. An in-place
    /// root reads no ignore rules.
    pub(crate) async fn checkout_ignored_paths(
        &self,
        verified: &VerifiedCheckout,
    ) -> Result<IgnoredPaths, CodingError> {
        if self.is_in_place() {
            return Ok(IgnoredPaths::none());
        }
        self.service
            .ignored_paths(
                Path::new(verified.path().as_str()),
                &saya_agent::CancellationToken::new(),
                DEFAULT_DELTA_DEADLINE,
            )
            .await
            .map_err(CodingError::owner)
    }
}

/// The checkout id the job's own private record names, or `None` when no record
/// exists yet. Read-only, and the one reader of that record's shape: the job's
/// verification and the operator's rewind both resolve the same id here.
pub(crate) fn read_root_checkout(
    records: &std::path::Path,
) -> Result<Option<ManagedCheckoutId>, CodingError> {
    let record = records.join("root-checkout.json");
    if !record.exists() {
        return Ok(None);
    }
    Ok(Some(RootCheckoutRecord::read(&record)?.checkout_id))
}

/// What one verified checkout already changed against the snapshot it was
/// created from, one entry per path in the delta's own order (sorted by path).
/// The job's own readiness reads and the operator's rewind share this one
/// computation.
pub(crate) fn checkout_changes(
    artifacts: &saya_harness::coding::snapshot::SnapshotArtifacts,
    verified: &VerifiedCheckout,
    ignored: &IgnoredPaths,
    cancellation: &saya_agent::CancellationToken,
) -> Result<Vec<RootChange>, CodingError> {
    let snapshot = artifacts
        .read_snapshot(verified.snapshot_id())
        .map_err(CodingError::owner)?;
    let delta = compute_delta(
        verified,
        &snapshot,
        ignored,
        cancellation,
        DEFAULT_DELTA_DEADLINE,
    )
    .map_err(CodingError::owner)?;
    Ok(delta
        .entries()
        .iter()
        .map(|entry| RootChange {
            path: entry.path().to_owned(),
            kind: entry.kind(),
            content_sha256: entry.content_sha256().cloned(),
            bytes: entry.bytes(),
        })
        .collect())
}
