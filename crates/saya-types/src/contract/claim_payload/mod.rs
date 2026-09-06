//! The claim payload contract: the `ClaimPayload` value a fact stores.
//!
//! The type definitions are this module's public surface; the validating
//! constructors live in `constructors` and the read-side derivations a
//! reconciler uses (kind discriminator, referenced columns, tombstoning,
//! column snapshots) live in `derive`.

mod constructors;
mod derive;

use serde::{Deserialize, Serialize};

use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::contract::identity::DatabaseObjectRef;

// Imported so the payload module's namespace — which its test modules pull in
// wholesale with `use super::*` — carries the column bound and the contract
// error the constructors and tests reference. The module's own type
// definitions don't use them directly; the constructors in `constructors`
// import what they need themselves.
#[allow(unused_imports)]
use crate::contract::{claim::MAX_REFERENCED_COLUMNS, error::ContractError};

/// A referenced column snapshotted at claim time: its name plus the type and
/// nullability the connector reported when the claim was made.
///
/// Phase 5a persists this so a later schema change can tell a *retyped*
/// referenced column (the claim may now be wrong) from an *unrelated* column
/// changing elsewhere (the claim is fine). A claim proposed without a live
/// table stores no snapshot; `data_type` is then empty and `nullable` is
/// `false`, which the reconciler treats as "unknown" rather than "matched".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferencedColumn {
    pub name: String,
    /// The column's type as the connector reported it when the claim was made.
    /// Empty for old `["a","b"]`-shape rows upgraded in place by the store.
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
// Every variant is `#[non_exhaustive]` as well as the enum. Without it another
// crate could write `ClaimPayload::TableDescription { text }` directly and skip
// the validation below, which is the only thing keeping oversized and
// control-character-bearing text out of a rendered context block.
pub enum ClaimPayload {
    #[non_exhaustive]
    TableDescription { text: String },
    #[non_exhaustive]
    TableAlias { alias: String },
    /// A directive claim: what a single row represents. Carries an optional
    /// `reason` so a model that would otherwise argue with the grain reads the
    /// justification (spec: claim-reasons). The reason is validated free text,
    /// bounded and control-char-stripped the way `TableDescription`'s `text`
    /// is; `None` is the state of every claim written before the field existed.
    #[non_exhaustive]
    TableGrain {
        description: String,
        #[serde(default)]
        reason: Option<String>,
    },
    #[non_exhaustive]
    ColumnDescription { column: String, text: String },
    /// A directive claim: the role a column plays. Carries an optional `reason`
    /// (see [`Self::TableGrain`]).
    #[non_exhaustive]
    ColumnRole {
        column: String,
        role: ColumnRole,
        #[serde(default)]
        reason: Option<String>,
    },
    /// A directive claim: the column to use as the default time dimension.
    /// Carries an optional `reason` (see [`Self::TableGrain`]).
    #[non_exhaustive]
    DefaultTimeColumn {
        column: String,
        #[serde(default)]
        reason: Option<String>,
    },
    #[non_exhaustive]
    Relationship {
        target: DatabaseObjectRef,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        cardinality: Cardinality,
    },
    /// A join condition the user taught, stored against the local table. The
    /// joined table is `target` (a qualified name in the same profile as the
    /// claim's object); `local_columns` and `target_columns` are the paired
    /// equi-join keys (both empty when the rule is a predicate-only join no
    /// declared constraint describes). `condition` is the full join condition
    /// as free text — it carries the extra predicate or soft-delete filter a
    /// real join adds on top of the keys, and is the only field a model or user
    /// could paste a credential into, so `blanked` empties it.
    #[non_exhaustive]
    JoinRule {
        target: String,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        condition: String,
        #[serde(default)]
        reason: Option<String>,
    },
    /// A business metric defined over the table the claim is stored against.
    /// `name` is the metric's handle, `definition` the formula as free text,
    /// and `columns` the underlying columns the binding tracks so the fact is
    /// invalidated when one of them disappears. `definition` is the
    /// secret-bearing field; `name` and `columns` are structural identity a
    /// tombstone keeps.
    #[non_exhaustive]
    MetricDefinition {
        name: String,
        definition: String,
        columns: Vec<String>,
        #[serde(default)]
        reason: Option<String>,
    },
}

#[cfg(test)]
#[path = "../claim_payload_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../claim_payload_property_tests.rs"]
mod property_tests;
