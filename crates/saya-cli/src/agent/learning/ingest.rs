//! Anti-self-reinforcement filtering and store ingestion — spec F Chunk 3.
//!
//! Enforces Safety Property 4 (drops redundant inferences on already supplied claims)
//! and persists verified proposals to [`KnowledgeItemStore`].

use crate::agent::learning::resolver::ResolvedProposal;
use crate::agent::learning::turn_record::SuppliedContractDto;
use crate::agent::recall_context::claim_value;
use crate::contracts::SuppliedContract;
use saya_agent::ProposedClaimDto;
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, KnowledgeStoreError};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, ContractError, DatabaseObjectRef,
    KnowledgeSlot, KnowledgeState, SchemaFingerprint,
};
use thiserror::Error;

/// Errors occurring during proposal ingestion.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum IngestionError {
    #[error("failed to serialize schema binding: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("store error: {0}")]
    Store(#[from] KnowledgeStoreError),
    #[error("invalid claim id: {0}")]
    ClaimId(#[from] ContractError),
}

/// Filters proposals against supplied contracts to prevent self-reinforcement (Safety Property 4).
///
/// - Retains `UserExplicit` proposals unconditionally (permits user corrections).
/// - For `AssistantInferred` proposals:
///   - Drops the proposal if an identical (slot, value) claim was already supplied for this object.
///   - Drops the proposal if the slot is single-valued and a claim for that slot is already in `supplied`.
#[allow(dead_code)]
pub fn filter_anti_self_reinforcement(
    proposals: Vec<ResolvedProposal>,
    supplied: &[SuppliedContract],
) -> Vec<ResolvedProposal> {
    proposals
        .into_iter()
        .filter(|prop| {
            if prop.source == ClaimOrigin::UserExplicit {
                return true;
            }

            let matching_contract = supplied.iter().find(|c| {
                c.profile.eq_ignore_ascii_case(&prop.profile_name)
                    && (c.object.eq_ignore_ascii_case(&prop.object.qualified_name())
                        || c.object.eq_ignore_ascii_case(prop.object.object()))
            });

            let Some(contract) = matching_contract else {
                return true;
            };

            let (prop_col, prop_val) = claim_value(&prop.value);
            let prop_kind = prop.value.kind();

            // 1. Redundant exact duplicate check
            let is_duplicate = contract.claims.iter().any(|c| {
                c.kind == prop_kind
                    && c.column.as_deref() == prop_col.as_deref()
                    && c.value.trim() == prop_val.trim()
            });
            if is_duplicate {
                return false;
            }

            // 2. Single-valued slot replacement suppression
            if prop.slot.cardinality().is_single() {
                let slot_already_present = contract
                    .claims
                    .iter()
                    .any(|c| c.kind == prop_kind && c.column.as_deref() == prop_col.as_deref());
                if slot_already_present {
                    return false;
                }
            }

            true
        })
        .collect()
}

/// Filters proposals against supplied contract DTOs (for integration with [`TurnRecord`]).
#[allow(dead_code)]
pub fn filter_anti_self_reinforcement_dto(
    proposals: Vec<ResolvedProposal>,
    supplied: &[SuppliedContractDto],
) -> Vec<ResolvedProposal> {
    proposals
        .into_iter()
        .filter(|prop| {
            if prop.source == ClaimOrigin::UserExplicit {
                return true;
            }

            let matching_contract = supplied.iter().find(|c| {
                c.profile.eq_ignore_ascii_case(&prop.profile_name)
                    && (c.object.eq_ignore_ascii_case(&prop.object.qualified_name())
                        || c.object.eq_ignore_ascii_case(prop.object.object()))
            });

            let Some(contract) = matching_contract else {
                return true;
            };

            let (prop_col, prop_val) = claim_value(&prop.value);
            let prop_kind = prop.value.kind();

            let is_duplicate = contract.claims.iter().any(|c| {
                c.kind == prop_kind
                    && c.column.as_deref() == prop_col.as_deref()
                    && c.value.trim() == prop_val.trim()
            });
            if is_duplicate {
                return false;
            }

            if prop.slot.cardinality().is_single() {
                let slot_already_present = contract
                    .claims
                    .iter()
                    .any(|c| c.kind == prop_kind && c.column.as_deref() == prop_col.as_deref());
                if slot_already_present {
                    return false;
                }
            }

            true
        })
        .collect()
}

/// Ingests resolved proposals into the [`KnowledgeItemStore`] and returns [`ProposedClaimDto`]s.
#[allow(dead_code)]
pub async fn ingest_proposals(
    store: &dyn KnowledgeItemStore,
    proposals: Vec<ResolvedProposal>,
    fingerprint: SchemaFingerprint,
) -> Result<Vec<ProposedClaimDto>, IngestionError> {
    let mut out = Vec::with_capacity(proposals.len());
    for proposal in proposals {
        let schema_binding_json = serde_json::to_string(&proposal.schema_binding)?;
        let req = KnowledgeItemRequest {
            object: proposal.object.clone(),
            slot: proposal.slot.clone(),
            value: proposal.value.clone(),
            source: proposal.source,
            state: proposal.state,
            schema_binding_json,
            fingerprint: fingerprint.clone(),
        };
        store.put_knowledge_item(req).await?;

        let (column, value) = claim_value(&proposal.value);
        let kind = proposal.value.kind().to_string();
        let claim_id = derive_claim_id(&proposal.object, &proposal.slot, &proposal.value)?;
        let status = match proposal.state {
            KnowledgeState::Active => ClaimStatus::Confirmed,
            KnowledgeState::Pending => ClaimStatus::Candidate,
            KnowledgeState::Dismissed => ClaimStatus::Rejected,
            _ => ClaimStatus::Candidate,
        };

        out.push(ProposedClaimDto {
            claim_id,
            profile: proposal.profile_name,
            object: proposal.object.qualified_name(),
            kind,
            value,
            column,
            status,
        });
    }
    Ok(out)
}

fn derive_claim_id(
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
    value: &ClaimPayload,
) -> Result<ClaimId, IngestionError> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(object.profile().as_str().as_bytes());
    hasher.update(object.catalog().as_bytes());
    hasher.update(object.schema().as_bytes());
    hasher.update(object.object().as_bytes());
    hasher.update(slot.as_str().as_bytes());
    if let Ok(v_json) = serde_json::to_string(value) {
        hasher.update(v_json.as_bytes());
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    let id_str = format!("ki-{}", &hex[..32]);
    ClaimId::parse(&id_str).map_err(IngestionError::ClaimId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::SuppliedClaim;
    use saya_store::SqliteStateStore;
    use saya_types::{ColumnRequirement, DatabaseObjectKind, ProfileIdentity, SchemaBinding};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "saya-ingest-{label}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_identity() -> ProfileIdentity {
        ProfileIdentity::parse("p-2222222222222222222222222222222222222222222222222222222222222222")
            .unwrap()
    }

    fn test_object(name: &str) -> DatabaseObjectRef {
        DatabaseObjectRef::new(
            test_identity(),
            "catalog",
            "public",
            name,
            DatabaseObjectKind::Table,
        )
        .unwrap()
    }

    #[test]
    fn test_anti_self_reinforcement_drops_supplied_duplicate_inference() {
        let obj = test_object("orders");
        let supplied = vec![SuppliedContract {
            profile: "primary".to_string(),
            object: obj.qualified_name(),
            schema_state: "current",
            claims: vec![SuppliedClaim {
                claim_id: ClaimId::parse("c-1").unwrap(),
                kind: "default_time_column",
                value: "created_at".to_string(),
                column: None,
                status: ClaimStatus::Confirmed,
            }],
        }];

        // Inferred proposal with exact matching slot and value -> dropped
        let duplicate_inferred = ResolvedProposal {
            object: obj.clone(),
            profile_name: "primary".to_string(),
            slot: KnowledgeSlot::TableDefaultTime,
            value: ClaimPayload::default_time_column("created_at", None).unwrap(),
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding: SchemaBinding::Column {
                column: "created_at".to_string(),
                requirement: ColumnRequirement::Time,
            },
        };

        let filtered = filter_anti_self_reinforcement(vec![duplicate_inferred], &supplied);
        assert!(filtered.is_empty(), "duplicate inference must be dropped");
    }

    #[test]
    fn test_anti_self_reinforcement_permits_user_explicit_correction() {
        let obj = test_object("orders");
        let supplied = vec![SuppliedContract {
            profile: "primary".to_string(),
            object: obj.qualified_name(),
            schema_state: "current",
            claims: vec![SuppliedClaim {
                claim_id: ClaimId::parse("c-1").unwrap(),
                kind: "default_time_column",
                value: "created_at".to_string(),
                column: None,
                status: ClaimStatus::Confirmed,
            }],
        }];

        // User explicit assertion with matching or updated value -> retained
        let user_prop = ResolvedProposal {
            object: obj.clone(),
            profile_name: "primary".to_string(),
            slot: KnowledgeSlot::TableDefaultTime,
            value: ClaimPayload::default_time_column("updated_at", None).unwrap(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding: SchemaBinding::Column {
                column: "updated_at".to_string(),
                requirement: ColumnRequirement::Time,
            },
        };

        let filtered = filter_anti_self_reinforcement(vec![user_prop.clone()], &supplied);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].state, KnowledgeState::Active);
    }

    #[test]
    fn test_anti_self_reinforcement_suppresses_replacement_of_single_valued_slot() {
        let obj = test_object("orders");
        let supplied = vec![SuppliedContract {
            profile: "primary".to_string(),
            object: obj.qualified_name(),
            schema_state: "current",
            claims: vec![SuppliedClaim {
                claim_id: ClaimId::parse("c-1").unwrap(),
                kind: "default_time_column",
                value: "created_at".to_string(),
                column: None,
                status: ClaimStatus::Confirmed,
            }],
        }];

        // Inferred proposal targeting single-valued slot already active in supplied -> dropped
        let different_time_inferred = ResolvedProposal {
            object: obj.clone(),
            profile_name: "primary".to_string(),
            slot: KnowledgeSlot::TableDefaultTime,
            value: ClaimPayload::default_time_column("shipped_at", None).unwrap(),
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding: SchemaBinding::Column {
                column: "shipped_at".to_string(),
                requirement: ColumnRequirement::Time,
            },
        };

        let filtered = filter_anti_self_reinforcement(vec![different_time_inferred], &supplied);
        assert!(
            filtered.is_empty(),
            "inferred replacement of active single-valued slot must be suppressed"
        );
    }

    #[test]
    fn test_anti_self_reinforcement_allows_new_inferred_slot_or_object() {
        let obj = test_object("orders");
        let supplied = vec![SuppliedContract {
            profile: "primary".to_string(),
            object: obj.qualified_name(),
            schema_state: "current",
            claims: vec![SuppliedClaim {
                claim_id: ClaimId::parse("c-1").unwrap(),
                kind: "default_time_column",
                value: "created_at".to_string(),
                column: None,
                status: ClaimStatus::Confirmed,
            }],
        }];

        // Inferred proposal for a DIFFERENT slot (e.g. TableAlias) -> retained
        let alias_prop = ResolvedProposal {
            object: obj.clone(),
            profile_name: "primary".to_string(),
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("customer_orders").unwrap(),
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding: SchemaBinding::Table,
        };

        // Inferred proposal for a DIFFERENT object -> retained
        let other_obj = test_object("line_items");
        let other_prop = ResolvedProposal {
            object: other_obj,
            profile_name: "primary".to_string(),
            slot: KnowledgeSlot::TableDefaultTime,
            value: ClaimPayload::default_time_column("created_at", None).unwrap(),
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding: SchemaBinding::Column {
                column: "created_at".to_string(),
                requirement: ColumnRequirement::Time,
            },
        };

        let filtered =
            filter_anti_self_reinforcement(vec![alias_prop.clone(), other_prop.clone()], &supplied);
        assert_eq!(filtered.len(), 2);
    }

    #[tokio::test]
    async fn test_ingest_proposals_persists_to_sqlite_store() {
        let root = temp_root("persist");
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        let obj = test_object("orders");
        let proposal = ResolvedProposal {
            object: obj.clone(),
            profile_name: "primary".to_string(),
            slot: KnowledgeSlot::TableGrain,
            value: ClaimPayload::table_grain("one row per order", None).unwrap(),
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding: SchemaBinding::Table,
        };

        let fingerprint = SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap();
        let dtos = ingest_proposals(&store, vec![proposal], fingerprint)
            .await
            .unwrap();

        assert_eq!(dtos.len(), 1);
        assert_eq!(dtos[0].profile, "primary");
        assert_eq!(dtos[0].object, obj.qualified_name());
        assert_eq!(dtos[0].kind, "table_grain");
        assert_eq!(dtos[0].value, "one row per order");
        assert_eq!(dtos[0].status, ClaimStatus::Candidate);

        let persisted = store.knowledge_for_object(&obj).await.unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].slot, KnowledgeSlot::TableGrain);
        assert_eq!(persisted[0].state, KnowledgeState::Pending);
        assert_eq!(persisted[0].source, ClaimOrigin::AssistantInferred);
    }

    #[tokio::test]
    async fn test_ingest_proposals_returns_dtos_for_persisted_items() {
        let root = temp_root("dtos");
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);

        let obj = test_object("products");
        let proposal = ResolvedProposal {
            object: obj.clone(),
            profile_name: "analytics".to_string(),
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("catalog_items").unwrap(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding: SchemaBinding::Table,
        };

        let fingerprint = SchemaFingerprint::from_parts(1, &"b".repeat(64)).unwrap();
        let dtos = ingest_proposals(&store, vec![proposal], fingerprint)
            .await
            .unwrap();

        assert_eq!(dtos.len(), 1);
        let dto = &dtos[0];
        assert_eq!(dto.profile, "analytics");
        assert_eq!(dto.object, obj.qualified_name());
        assert_eq!(dto.kind, "table_alias");
        assert_eq!(dto.value, "catalog_items");
        assert_eq!(dto.column, None);
        assert_eq!(dto.status, ClaimStatus::Confirmed);
    }

    #[test]
    fn test_anti_self_reinforcement_dto_variant() {
        use crate::agent::learning::turn_record::SuppliedClaimDto;

        let obj = test_object("orders");
        let supplied_dto = vec![SuppliedContractDto {
            profile: "primary".to_string(),
            object: obj.qualified_name(),
            schema_state: "current".to_string(),
            claims: vec![SuppliedClaimDto {
                claim_id: "c-1".to_string(),
                kind: "default_time_column".to_string(),
                value: "created_at".to_string(),
                column: None,
                status: "confirmed".to_string(),
            }],
        }];

        let duplicate_inferred = ResolvedProposal {
            object: obj.clone(),
            profile_name: "primary".to_string(),
            slot: KnowledgeSlot::TableDefaultTime,
            value: ClaimPayload::default_time_column("created_at", None).unwrap(),
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding: SchemaBinding::Column {
                column: "created_at".to_string(),
                requirement: ColumnRequirement::Time,
            },
        };

        let filtered = filter_anti_self_reinforcement_dto(vec![duplicate_inferred], &supplied_dto);
        assert!(filtered.is_empty(), "dto variant drops duplicate inference");
    }
}
