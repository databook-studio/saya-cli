use crate::StoreError;
use crate::contracts::events::{ContractEvent, ForgetReason};
use async_trait::async_trait;
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, DatabaseObjectRef, ProfileIdentity,
    ReferencedColumn, SchemaFingerprint, Table,
};

pub const MAX_CLAIM_PAYLOAD_BYTES: usize = 4096;
pub const MAX_CLAIMS_PER_OBJECT: usize = 128;
pub const MAX_EVIDENCE_PER_CLAIM: usize = 32;
pub const MAX_LISTED_OBJECTS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContractObjectId(String);

impl ContractObjectId {
    pub(crate) fn from_inner(value: String) -> Self {
        Self(value)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContractObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeduplicationKey(String);

impl DeduplicationKey {
    pub(crate) fn from_inner(value: String) -> Self {
        Self(value)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeduplicationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub id: ContractObjectId,
    pub object: DatabaseObjectRef,
    pub fingerprint: SchemaFingerprint,
    pub fingerprint_version: u32,
    pub first_seen_unix_ms: i64,
    pub last_seen_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredClaim {
    pub id: ClaimId,
    pub object: DatabaseObjectRef,
    pub payload: Option<ClaimPayload>,
    pub origin: ClaimOrigin,
    pub status: ClaimStatus,
    pub schema_fingerprint: SchemaFingerprint,
    /// Per-referenced-column snapshots persisted for Phase 5 drift detection.
    /// A claim proposed without a live schema stores name-only snapshots
    /// (empty `data_type`), which the reconciler treats as unknown rather than
    /// matched — see `store_decode` for the old `["a","b"]` shape upgrade.
    pub referenced_columns: Vec<ReferencedColumn>,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
    pub last_verified_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EvidenceKind {
    ExplicitUserStatement,
    SuccessfulReadQuery,
    RepeatedObservation,
    ReviewedImport,
    ManualConfirmation,
}

impl EvidenceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitUserStatement => "explicit_user_statement",
            Self::SuccessfulReadQuery => "successful_read_query",
            Self::RepeatedObservation => "repeated_observation",
            Self::ReviewedImport => "reviewed_import",
            Self::ManualConfirmation => "manual_confirmation",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "explicit_user_statement" => Some(Self::ExplicitUserStatement),
            "successful_read_query" => Some(Self::SuccessfulReadQuery),
            "repeated_observation" => Some(Self::RepeatedObservation),
            "reviewed_import" => Some(Self::ReviewedImport),
            "manual_confirmation" => Some(Self::ManualConfirmation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimEvidence {
    pub kind: EvidenceKind,
    pub session_id: Option<String>,
    pub turn_ordinal: Option<u32>,
    pub observed_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposeClaim {
    pub object: DatabaseObjectRef,
    pub fingerprint: SchemaFingerprint,
    pub payload: ClaimPayload,
    pub origin: ClaimOrigin,
    pub initial_status: ClaimStatus,
    pub evidence: Option<ClaimEvidence>,
    /// Snapshots of the claim's referenced columns resolved against the live
    /// table. The caller — which has the live schema — builds these via
    /// `ClaimPayload::referenced_column_snapshots`. A caller with no live
    /// schema passes an empty vec; the store persists no type, and the
    /// reconciler treats the column as unknown.
    pub referenced_columns: Vec<ReferencedColumn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposeOutcome {
    Stored(ClaimId),
    Duplicate { id: ClaimId, status: ClaimStatus },
}

#[async_trait]
pub trait ContractStore: Send + Sync {
    async fn upsert_object(
        &self,
        object: &DatabaseObjectRef,
        fingerprint: &SchemaFingerprint,
    ) -> Result<ContractObjectId, StoreError>;
    async fn propose_claim(&self, request: ProposeClaim) -> Result<ProposeOutcome, StoreError>;
    async fn get_claim(&self, id: &ClaimId) -> Result<Option<StoredClaim>, StoreError>;
    async fn list_claims(
        &self,
        object: &DatabaseObjectRef,
        statuses: &[ClaimStatus],
    ) -> Result<Vec<StoredClaim>, StoreError>;
    async fn list_objects(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<StoredObject>, StoreError>;
    async fn confirm_claim(&self, id: &ClaimId) -> Result<StoredClaim, StoreError>;
    /// Reconfirm a claim against a live table, rewriting its fingerprint and
    /// referenced-column snapshots in the same transaction as the status flip.
    /// The counterpart to [`ContractStore::confirm_claim`] for a Stale claim:
    /// a status-only flip left the stored digest untouched, so the next read
    /// returned Stale again. Refuses with [`StoreError::Conflict`] when a
    /// referenced column is absent from `live_table` — never revive a claim
    /// against a schema its dependency vanished from.
    async fn revalidate_claim(
        &self,
        id: &ClaimId,
        live_table: &Table,
    ) -> Result<StoredClaim, StoreError>;
    async fn edit_claim(
        &self,
        id: &ClaimId,
        payload: ClaimPayload,
    ) -> Result<StoredClaim, StoreError>;
    async fn reject_claim(&self, id: &ClaimId) -> Result<StoredClaim, StoreError>;
    async fn forget_claim(&self, id: &ClaimId, reason: ForgetReason) -> Result<(), StoreError>;
    async fn mark_stale(&self, id: &ClaimId) -> Result<StoredClaim, StoreError>;
    async fn claim_events(
        &self,
        id: &ClaimId,
        limit: usize,
    ) -> Result<Vec<ContractEvent>, StoreError>;
    /// The number of evidence rows attached to a claim. Returns a bare count
    /// only — never the rows themselves, which carry session ids and turn
    /// ordinals the queue does not need. An unknown id is `NotFound`, not `0`:
    /// a missing claim is not an empty evidence set.
    async fn evidence_count(&self, id: &ClaimId) -> Result<usize, StoreError>;
}
