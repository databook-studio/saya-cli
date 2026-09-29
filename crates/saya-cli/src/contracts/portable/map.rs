//! Mapping validated context items onto one profile's schema (B2c invariant 2).
//!
//! Each item's logical object — and, for target-bearing payloads, the target —
//! resolves against the profile's schema tree under the identity rules in
//! `resolve`. A mapped item becomes a pending batch item with the team-file
//! origin and the schema binding its slot derives; an item that resolves to
//! nothing or carries a claim the constructors refuse becomes an
//! `unavailable` entry, never an error, so one bad object cannot hide the rest.
//!
//! A relationship payload files as the keyed join rule — the one
//! relationship-shaped slot the store can hold. Nothing is invented: the
//! condition is the column equality the relationship asserts, the target and
//! key columns travel verbatim, and the cardinality (no join-rule field) is
//! dropped.

use super::resolve::{Resolution, object_label, resolve_object, resolve_ref};
use saya_store::NewKnowledgeItem;
use saya_types::{
    ClaimOrigin, ClaimPayload, ContextError, ContextItem, DatabaseObjectRef, ProfileIdentity,
    SchemaBinding, SchemaFingerprint, SchemaTree,
};

/// One mapped item ready for the pending batch: the resolved object name for
/// the report (`label`), the filed slot's string form (`slot`), and the batch
/// item itself.
#[derive(Debug)]
pub(crate) struct PlannedItem {
    pub label: String,
    pub slot: String,
    pub item: NewKnowledgeItem,
}

/// An item that maps to nothing: reported, never written.
#[derive(Debug)]
pub(crate) struct Unavailability {
    pub label: String,
    pub reason: String,
}

#[derive(Default)]
pub(crate) struct MappedImport {
    pub planned: Vec<PlannedItem>,
    pub unavailable: Vec<Unavailability>,
}

/// Maps every document item against `tree`; per-item failures become
/// `unavailable` entries, never errors, so one bad object cannot hide the
/// rest. The caller validates the document before this runs and applies the
/// batch after, so nothing here touches the store.
pub(crate) fn map_items(
    items: &[ContextItem],
    tree: &SchemaTree,
    identity: &ProfileIdentity,
) -> MappedImport {
    let mut mapped = MappedImport::default();
    for item in items {
        match map_one(item, tree, identity) {
            Ok(planned) => mapped.planned.push(planned),
            Err(reason) => mapped.unavailable.push(Unavailability {
                label: object_label(&item.object),
                reason,
            }),
        }
    }
    mapped
}

fn map_one(
    item: &ContextItem,
    tree: &SchemaTree,
    identity: &ProfileIdentity,
) -> Result<PlannedItem, String> {
    let (catalog, schema, table) = match resolve_object(tree, &item.object) {
        Resolution::Resolved {
            catalog,
            schema,
            table,
        } => (catalog, schema, table),
        Resolution::Missing => {
            return Err("no matching object in this profile's schema".to_string());
        }
        Resolution::Ambiguous => {
            return Err("the name matches several objects in this profile's schema".to_string());
        }
    };
    let object = DatabaseObjectRef::new(
        identity.clone(),
        catalog,
        schema,
        &table.name,
        item.object.kind,
    )
    .map_err(|_| "the object name is not fileable".to_string())?;
    let claim = claim_of(item, tree, identity)?;
    let slot = crate::contracts::args::slot_for_payload(&claim)
        .ok_or_else(|| "this claim kind has no fileable slot".to_string())?;
    let binding = SchemaBinding::derive(&slot, &claim)
        .ok_or_else(|| "this claim's slot and payload disagree".to_string())?;
    let binding_json = serde_json::to_string(&binding)
        .map_err(|_| "the claim could not be serialised".to_string())?;
    Ok(PlannedItem {
        label: object.qualified_name(),
        slot: slot.as_str(),
        item: NewKnowledgeItem {
            object,
            slot,
            value: claim,
            source: ClaimOrigin::TeamFile,
            schema_binding_json: binding_json,
            fingerprint: SchemaFingerprint::of_table(item.object.kind, table),
        },
    })
}

/// Rebuilds the claim, resolving target-bearing payloads against the same
/// tree; a resolved relationship files as the keyed join rule.
fn claim_of(
    item: &ContextItem,
    tree: &SchemaTree,
    identity: &ProfileIdentity,
) -> Result<ClaimPayload, String> {
    let claim = item
        .payload
        .clone()
        .into_claim(|target| resolve_ref(tree, identity, target))
        .map_err(|error| match error {
            ContextError::UnresolvedTarget => {
                "the claim's target object does not resolve in this profile's schema".to_string()
            }
            // `ContextError` is `#[non_exhaustive]`; every other refusal is
            // the claim content itself, which is never echoed.
            _ => "the claim payload could not be rebuilt".to_string(),
        })?;
    match claim {
        ClaimPayload::Relationship {
            target,
            local_columns,
            target_columns,
            ..
        } => {
            let condition = column_pairs(&local_columns, &target_columns)
                .ok_or_else(|| "the relationship's columns do not pair".to_string())?;
            ClaimPayload::join_rule(
                target.qualified_name(),
                local_columns,
                target_columns,
                condition,
                None,
            )
            .map_err(|_| "the relationship could not be filed as a join rule".to_string())
        }
        other => Ok(other),
    }
}

/// The equality conditions one keyed join asserts, paired positionally.
fn column_pairs(local: &[String], target: &[String]) -> Option<String> {
    let conditions: Vec<String> = local
        .iter()
        .zip(target.iter())
        .map(|(l, t)| format!("{l} = {t}"))
        .collect();
    if conditions.is_empty() {
        return None;
    }
    Some(conditions.join(" AND "))
}
