//! Spec D-2 — one computed validity over the new state vocabulary.
//!
//! `KnowledgeValidity` is the verdict a claim earns against the schema known
//! for its profile. It supersedes [`crate::contracts::view::ContractSchemaState`]
//! without deleting it: nothing adopts the new vocabulary yet, so the old type
//! and [`schema_state_for`](super::validity::schema_state_for) keep driving
//! recall/show/reconcile unchanged. This module is the pure vocabulary the
//! adopting slice will switch to.
//!
//! Why a separate enum at all: `ContractSchemaState::Stale` is *computed*, but
//! `ClaimStatus::Stale` is *persisted* — the same fact had two homes and they
//! could disagree. D-1 introduced [`KnowledgeState`] with no `Stale` variant
//! because staleness is a read-time computation, not a stored state. This
//! module owns that computation under the new names: `Invalid` is what a gone
//! referenced column reads as, and it is never persisted.
//!
//! The logic is not reimplemented here. [`knowledge_validity_for`] calls the
//! existing [`schema_state_for`](super::validity::schema_state_for) — the same
//! availability gating, fingerprint-format gate, object lookup, and drift
//! classification — and translates its verdict into the new vocabulary. The
//! only thing added is the [`KnowledgeState`] dimension: a [`Dismissed`] claim
//! is withdrawn from use and so is never [`Valid`](KnowledgeValidity::Valid),
//! whatever the schema says.
//!
//! [`Dismissed`]: saya_types::KnowledgeState::Dismissed

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::validity::schema_state_for;
use crate::contracts::view::ContractSchemaState;
use saya_store::StoredClaim;
use saya_types::{ClaimStatus, KnowledgeState};

/// The computed validity of a claim against the schema known for its profile.
///
/// Supersedes [`ContractSchemaState`] one-for-one: `Valid` ← `Current`,
/// `NeedsReview` ← `NeedsReview`, `SchemaUnavailable` ← `LiveSchemaUnavailable`,
/// `Invalid` ← `Stale`. `Invalid` is the new name for the verdict that a
/// referenced column the claim depends on is gone — a computed fact, never
/// persisted (D-1 moved staleness out of the stored state).
///
/// Ordered worst-first for aggregation via [`Self::aggregate`]:
/// `Invalid` beats `SchemaUnavailable` beats `NeedsReview` beats `Valid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnowledgeValidity {
    /// The claim matches the schema it was made against (← `Current`).
    Valid,
    /// The schema moved in a way a human must judge (← `NeedsReview`). Not
    /// dropped on the model path: one unrelated column added to a wide table
    /// must not mute every claim on it.
    NeedsReview,
    /// No usable schema: the cache is missing, unreadable, or (on the model
    /// path) past the freshness bound (← `LiveSchemaUnavailable`). Never
    /// `Invalid` — an absent cache is not evidence a column is gone.
    SchemaUnavailable,
    /// A referenced column the claim depends on is gone, or changed in a way
    /// that can silently break it (← `Stale`). A contract that aggregates to
    /// `Invalid` is not safe for the model to read as a fact.
    Invalid,
}

impl KnowledgeValidity {
    /// A claim cannot be more reassuring than the worst claim in the same
    /// contract — one `Invalid` claim must flag the object, not hide behind a
    /// `Valid` sibling. Mirrors [`ContractSchemaState::aggregate`].
    pub(crate) fn aggregate(self, other: Self) -> Self {
        let rank = |state: Self| match state {
            Self::Invalid => 3,
            Self::SchemaUnavailable => 2,
            Self::NeedsReview => 1,
            Self::Valid => 0,
        };
        if rank(self) >= rank(other) {
            self
        } else {
            other
        }
    }
}

impl From<ContractSchemaState> for KnowledgeValidity {
    fn from(state: ContractSchemaState) -> Self {
        match state {
            ContractSchemaState::Current => Self::Valid,
            ContractSchemaState::NeedsReview => Self::NeedsReview,
            ContractSchemaState::LiveSchemaUnavailable => Self::SchemaUnavailable,
            ContractSchemaState::Stale => Self::Invalid,
        }
    }
}

/// The computed validity of `claim` under the new state vocabulary.
///
/// A [`KnowledgeState::Dismissed`] claim is withdrawn from use — it is
/// [`Invalid`](KnowledgeValidity::Invalid) without a schema lookup, because no
/// revalidation can revive it and it must never be read as a live fact. Every
/// other state is classified the same way [`schema_state_for`] already classifies
/// a stored claim, then translated into the new vocabulary. Reuses the existing
/// availability gating, fingerprint-format gate, object lookup, and drift rule
/// rather than reimplementing any of them.
///
/// Pure: no store, no clock, no I/O. `freshness` is an input, not a wall-clock
/// read, so the model path can pass [`SchemaFreshness::for_model`] with a
/// controlled stamp and the human path can pass [`SchemaFreshness::Unbounded`].
pub(crate) fn knowledge_validity_for(
    claim: &StoredClaim,
    state: KnowledgeState,
    availability: &SchemaAvailability,
    freshness: SchemaFreshness,
) -> KnowledgeValidity {
    // A dismissed claim is withdrawn: not recallable, not revivable by
    // revalidation. Whatever the schema says, it must not read `Valid` — and
    // `Invalid` is the verdict that keeps it out of the model's facts. This
    // fires before the schema lookup so a missing or unreadable cache cannot
    // upgrade a dismissed claim to `SchemaUnavailable` (which is kept, not
    // dropped, and would let a dismissed claim reach the model labelled).
    if matches!(state, KnowledgeState::Dismissed) {
        return KnowledgeValidity::Invalid;
    }
    schema_state_for(claim, availability, freshness).into()
}

/// The persisted lifecycle state a stored [`ClaimStatus`] maps to under D-1's
/// vocabulary. This is the mapping the adopting slice will use to translate the
/// statuses already in the store.
///
/// Written as a free function, not `impl From<ClaimStatus> for KnowledgeState`,
/// because both types live in `saya-types` (foreign to this crate) and the
/// orphan rule forbids the trait impl here. D-1 landed `KnowledgeState` as
/// final and the spec bars touching `saya-types`, so the mapping lives in the
/// crate that consumes it.
///
/// - `Candidate` → [`KnowledgeState::Pending`]: awaiting a confirmation step.
/// - `Confirmed` → [`KnowledgeState::Active`]: a usable, live fact.
/// - `Rejected` → [`KnowledgeState::Dismissed`]: a user-rejected value.
/// - `Forgotten` → [`KnowledgeState::Dismissed`]: withdrawn from use.
/// - `Stale` → [`KnowledgeState::Pending`]: a *computed* staleness that was
///   persisted. In the new vocabulary staleness is recomputed, not stored, so
///   the stored `Stale` is not trusted as a verdict — it is a claim held back
///   pending the revalidation `confirm` performs. Confirming a `Stale` claim
///   revalidates it (the binding P1 property), which is exactly `Pending`
///   semantics: an action resolves it to `Active` or refuses.
/// - `Contradicted` → [`KnowledgeState::Dismissed`]: two confirmed claims
///   disagreed about a single-valued property. `KnowledgeSlot` cardinality now
///   prevents the structural condition, and `Contradicted` is an obsolete
///   terminal — not revivable by revalidation (the contradiction is logical,
///   not schema-driven) and never usable, so it joins `Rejected`/`Forgotten` in
///   `Dismissed`.
pub(crate) fn knowledge_state_from_status(status: ClaimStatus) -> KnowledgeState {
    match status {
        ClaimStatus::Candidate => KnowledgeState::Pending,
        ClaimStatus::Confirmed => KnowledgeState::Active,
        ClaimStatus::Rejected | ClaimStatus::Forgotten | ClaimStatus::Contradicted => {
            KnowledgeState::Dismissed
        }
        ClaimStatus::Stale => KnowledgeState::Pending,
        // `ClaimStatus` is `#[non_exhaustive]`; every defined variant is named
        // above. An unknown future status must not default to a trusting state
        // (Active/Pending grant use), so fail closed to Dismissed.
        _ => KnowledgeState::Dismissed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
    use saya_types::{Column, DatabaseObjectKind, SchemaFingerprint, SchemaTree};

    /// A cache observed at "now" so the model-path freshness bound never fires —
    /// these tests exercise the table/fingerprint logic, not the age gate.
    const FRESH_NOW: i64 = 1_000_000;

    fn schema_with(table: saya_types::Table) -> SchemaTree {
        SchemaTree {
            databases: vec![saya_types::Database {
                name: "catalog".into(),
                schemas: vec![saya_types::Schema {
                    name: "public".into(),
                    tables: vec![table],
                }],
            }],
        }
    }

    fn base_claim() -> StoredClaim {
        StoredClaim {
            id: saya_types::ClaimId::parse("c-x").unwrap(),
            object: saya_types::DatabaseObjectRef::new(
                saya_types::ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap(),
                "catalog",
                "public",
                "orders",
                DatabaseObjectKind::Table,
            )
            .unwrap(),
            payload: None,
            origin: saya_types::ClaimOrigin::UserExplicit,
            status: saya_types::ClaimStatus::Confirmed,
            schema_fingerprint: SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap(),
            referenced_columns: vec![],
            created_unix_ms: 0,
            updated_unix_ms: 0,
            last_verified_unix_ms: None,
        }
    }

    fn table(cols: &[(&str, &str, bool)]) -> saya_types::Table {
        saya_types::Table {
            name: "orders".into(),
            columns: cols
                .iter()
                .map(|(name, ty, nullable)| Column {
                    name: (*name).into(),
                    data_type: (*ty).into(),
                    nullable: *nullable,
                })
                .collect(),
        }
    }

    /// A claim whose fingerprint matches `table`, so it classifies `Valid`
    /// against a schema containing exactly `table`.
    fn current_claim_for(table: saya_types::Table) -> StoredClaim {
        let mut claim = base_claim();
        claim.schema_fingerprint = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
        claim
    }

    // 1. Each ClaimStatus maps to the chosen KnowledgeState; Stale and
    //    Contradicted assert the §Deliverable-3 decision explicitly.
    #[test]
    fn claim_status_maps_to_knowledge_state() {
        assert_eq!(
            knowledge_state_from_status(ClaimStatus::Candidate),
            KnowledgeState::Pending
        );
        assert_eq!(
            knowledge_state_from_status(ClaimStatus::Confirmed),
            KnowledgeState::Active
        );
        assert_eq!(
            knowledge_state_from_status(ClaimStatus::Rejected),
            KnowledgeState::Dismissed
        );
        assert_eq!(
            knowledge_state_from_status(ClaimStatus::Forgotten),
            KnowledgeState::Dismissed
        );
        // Stale is a computed verdict that was persisted; in the new vocabulary
        // staleness is recomputed, so the stored status is not trusted — the
        // claim is held back pending revalidation, i.e. Pending. Confirming it
        // revalidates (the P1 property), which is the Pending→Active transition.
        assert_eq!(
            knowledge_state_from_status(ClaimStatus::Stale),
            KnowledgeState::Pending,
            "a persisted Stale is Pending, not a stored verdict"
        );
        // Contradicted is an obsolete terminal: slot cardinality now prevents
        // the structural condition, and no revalidation revives a logical
        // contradiction. It is withdrawn from use, so Dismissed.
        assert_eq!(
            knowledge_state_from_status(ClaimStatus::Contradicted),
            KnowledgeState::Dismissed,
            "a contradiction is a dismissed value, not a state"
        );
    }

    // 2. Missing schema → SchemaUnavailable. Unavailable → SchemaUnavailable.
    //    Neither → Invalid (an absent/unreadable cache is not evidence a column
    //    is gone — the P1 bug that already bit once).
    #[test]
    fn missing_or_unavailable_schema_is_schema_unavailable_not_invalid() {
        let claim = current_claim_for(table(&[("id", "bigint", false)]));
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Active,
                &SchemaAvailability::Missing,
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::SchemaUnavailable,
            "a missing cache is not evidence a column is gone"
        );
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Active,
                &SchemaAvailability::Unavailable,
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::SchemaUnavailable,
            "a store error is not evidence a column is gone"
        );
    }

    // 3. A stale-by-age cached schema beyond the freshness bound →
    //    SchemaUnavailable on the model path, not Valid.
    #[test]
    fn stale_cached_schema_beyond_freshness_is_schema_unavailable() {
        let t = table(&[("id", "bigint", false)]);
        let claim = current_claim_for(t.clone());
        let schema = schema_with(t);
        // Observed at 0; "now" is 25h later — past the 24h model-path bound.
        let now = 25 * 60 * 60 * 1000;
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Active,
                &SchemaAvailability::available(schema, 0),
                SchemaFreshness::for_model(now),
            ),
            KnowledgeValidity::SchemaUnavailable,
            "a stale-by-age cache cannot vouch for currency on the model path"
        );
    }

    // 4. A fingerprint in a format the running binary no longer uses →
    //    NeedsReview, never Valid.
    #[test]
    fn non_current_fingerprint_format_is_needs_review() {
        let mut claim = base_claim();
        claim.schema_fingerprint = SchemaFingerprint::from_parts(2, &"a".repeat(64)).unwrap();
        let schema = schema_with(table(&[("id", "bigint", false)]));
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Active,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::NeedsReview,
            "a non-current fingerprint format is not comparable"
        );
    }

    // 5. A dropped referenced column → Invalid; drift outside the claim's
    //    columns → NeedsReview.
    #[test]
    fn dropped_referenced_column_is_invalid_unrelated_drift_is_needs_review() {
        // A claim that references `amount`, made against a table that had it.
        let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
        let mut claim = current_claim_for(base.clone());
        claim.referenced_columns = vec![saya_types::ReferencedColumn {
            name: "amount".into(),
            data_type: "numeric".into(),
            nullable: false,
        }];

        // Drift that dropped `amount`: the referenced column is gone → Invalid.
        let dropped = schema_with(table(&[("id", "bigint", false)]));
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Active,
                &SchemaAvailability::available(dropped, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "a gone referenced column makes the claim invalid"
        );

        // Drift outside the claim's columns: `note` added, `amount` intact, but
        // the fingerprint moved → NeedsReview (the claim may still be true).
        let drifted = schema_with(table(&[
            ("id", "bigint", false),
            ("amount", "numeric", false),
            ("note", "text", true),
        ]));
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Active,
                &SchemaAvailability::available(drifted, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::NeedsReview,
            "drift in a column the claim does not depend on is needs review"
        );
    }

    // 6. Aggregation: one Invalid among Valid siblings yields Invalid.
    #[test]
    fn aggregate_one_invalid_among_valid_is_invalid() {
        let verdicts = [
            KnowledgeValidity::Valid,
            KnowledgeValidity::Invalid,
            KnowledgeValidity::Valid,
        ];
        let aggregate = verdicts
            .iter()
            .copied()
            .fold(KnowledgeValidity::Valid, KnowledgeValidity::aggregate);
        assert_eq!(
            aggregate,
            KnowledgeValidity::Invalid,
            "one invalid claim flags its object; it cannot hide behind a valid sibling"
        );
    }

    // 7. A Dismissed claim is never Valid regardless of schema.
    #[test]
    fn dismissed_claim_is_never_valid_regardless_of_schema() {
        let t = table(&[("id", "bigint", false)]);
        let claim = current_claim_for(t.clone());
        let schema = schema_with(t);
        // A schema the claim perfectly matches would classify Valid for an
        // Active claim — but a Dismissed claim is withdrawn, so it reads
        // Invalid without ever consulting the schema.
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Dismissed,
                &SchemaAvailability::available(schema, FRESH_NOW),
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "a dismissed claim is not a live fact, whatever the schema says"
        );
        // And the same holds when there is no schema at all: a Dismissed claim
        // must not upgrade to SchemaUnavailable (which is kept on the model
        // path) — it stays Invalid.
        assert_eq!(
            knowledge_validity_for(
                &claim,
                KnowledgeState::Dismissed,
                &SchemaAvailability::Missing,
                SchemaFreshness::for_model(FRESH_NOW),
            ),
            KnowledgeValidity::Invalid,
            "dismissal beats schema unavailability"
        );
    }
}
