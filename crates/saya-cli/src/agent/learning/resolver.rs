//! Proposal resolution and schema binding derivation — spec F Chunk 3.
//!
//! Maps extracted turn-scoped proposals (`T0..Tn`) back to fully-resolved
//! `DatabaseObjectRef`s and derives structural `SchemaBinding`s from the
//! paired `(KnowledgeSlot, ClaimPayload)`.

use crate::agent::learning::extractor_schema::{ExtractedProposal, ProposalOrigin};
use crate::agent::learning::turn_table::{TurnObjectId, TurnObjectTable};
use crate::connection::ConnectionRegistry;
use saya_types::{
    ClaimOrigin, ClaimPayload, ContractError, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaBinding,
};
use thiserror::Error;

/// A proposal fully resolved to a database object reference, verified profile,
/// and derived schema binding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ResolvedProposal {
    pub object: DatabaseObjectRef,
    pub profile_name: String,
    pub slot: KnowledgeSlot,
    pub value: ClaimPayload,
    pub source: ClaimOrigin,
    pub state: KnowledgeState,
    pub schema_binding: SchemaBinding,
}

/// Errors occurring during proposal resolution.
#[derive(Debug, Error, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ResolutionError {
    #[error("turn object id {0} not found in turn table")]
    ObjectIdNotFound(TurnObjectId),
    #[error("connection error resolving profile '{0}': {1}")]
    ConnectionError(String, String),
    #[error("connection entry for profile '{0}' has no profile id")]
    MissingProfileIdentity(String),
    #[error("invalid profile identity for '{0}': {1}")]
    InvalidProfileIdentity(String, String),
    #[error("invalid object reference '{0}': {1}")]
    InvalidObjectRef(String, String),
    #[error("failed to derive schema binding for slot and payload")]
    InvalidSchemaBinding,
}

/// Resolves an extracted turn-scoped proposal into a [`ResolvedProposal`].
///
/// 1. Maps `extracted.object_id` to `(profile_name, qualified_name)` via `table`.
/// 2. Resolves the profile connection via `registry` and validates `ProfileIdentity`.
/// 3. Derives `SchemaBinding` from `(slot, value)`; returns `Err` if invalid.
/// 4. Maps origin:
///    - `ProposalOrigin::UserExplicit` -> `(KnowledgeState::Active, ClaimOrigin::UserExplicit)`
///    - `ProposalOrigin::AssistantInferred` -> `(KnowledgeState::Pending, ClaimOrigin::AssistantInferred)`
#[allow(dead_code)]
pub fn resolve_proposal(
    extracted: ExtractedProposal,
    table: &TurnObjectTable,
    registry: &ConnectionRegistry,
) -> Result<ResolvedProposal, ResolutionError> {
    let entry = table
        .get_by_id(&extracted.object_id)
        .ok_or(ResolutionError::ObjectIdNotFound(extracted.object_id))?;

    let conn_entry = registry
        .resolve(Some(&entry.profile))
        .map_err(|e| ResolutionError::ConnectionError(entry.profile.clone(), e.to_string()))?;

    let profile_id_str = conn_entry
        .profile_id
        .as_deref()
        .ok_or_else(|| ResolutionError::MissingProfileIdentity(entry.profile.clone()))?;

    let profile_identity = ProfileIdentity::parse(profile_id_str).map_err(|e| {
        ResolutionError::InvalidProfileIdentity(entry.profile.clone(), e.to_string())
    })?;

    let object = parse_database_object(&profile_identity, &entry.qualified_name).map_err(|e| {
        ResolutionError::InvalidObjectRef(entry.qualified_name.clone(), e.to_string())
    })?;

    let schema_binding = SchemaBinding::derive(&extracted.slot, &extracted.value)
        .ok_or(ResolutionError::InvalidSchemaBinding)?;

    let (state, source) = match extracted.origin {
        ProposalOrigin::UserExplicit => (KnowledgeState::Active, ClaimOrigin::UserExplicit),
        ProposalOrigin::AssistantInferred => {
            (KnowledgeState::Pending, ClaimOrigin::AssistantInferred)
        }
    };

    Ok(ResolvedProposal {
        object,
        profile_name: entry.profile.clone(),
        slot: extracted.slot,
        value: extracted.value,
        source,
        state,
        schema_binding,
    })
}

fn parse_database_object(
    profile: &ProfileIdentity,
    qualified_name: &str,
) -> Result<DatabaseObjectRef, ContractError> {
    let parts: Vec<&str> = qualified_name
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let (cat, sch, obj) = match parts.as_slice() {
        [cat, sch, obj] => (*cat, *sch, *obj),
        [sch, obj] => ("default", *sch, *obj),
        [obj] => ("default", "public", *obj),
        _ => return Err(ContractError::EmptyName),
    };
    DatabaseObjectRef::new(profile.clone(), cat, sch, obj, DatabaseObjectKind::Table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ConnectionEntry;
    use async_trait::async_trait;
    use saya_connectors::DatabaseConnector;
    use saya_types::{
        ColumnRequirement, ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect,
    };

    struct TestConnector;

    #[async_trait]
    impl DatabaseConnector for TestConnector {
        fn dialect(&self) -> SqlDialect {
            SqlDialect::DuckDb
        }
        async fn connect(&self) -> Result<(), ConnectionError> {
            Ok(())
        }
        async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
            Ok(SchemaTree::default())
        }
        async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
            Ok(QueryResult {
                columns: vec!["id".into()],
                rows: vec![serde_json::json!([1])],
                row_count: 1,
                truncated: false,
                executed_sql: req.sql,
            })
        }
    }

    fn test_registry(name: &str, identity: &ProfileIdentity) -> ConnectionRegistry {
        let mut reg = ConnectionRegistry::new(name);
        reg.insert(
            name,
            ConnectionEntry {
                connector: Box::new(TestConnector),
                dialect: SqlDialect::DuckDb,
                profile_id: Some(identity.as_str().to_string()),
            },
        );
        reg
    }

    fn test_identity() -> ProfileIdentity {
        ProfileIdentity::parse("p-1111111111111111111111111111111111111111111111111111111111111111")
            .unwrap()
    }

    #[test]
    fn test_resolve_proposal_derives_schema_binding() {
        let identity = test_identity();
        let registry = test_registry("primary", &identity);
        let mut table = TurnObjectTable::new();
        let id = table
            .register("primary", "analytics.public.orders", &[])
            .unwrap();

        let extracted = ExtractedProposal {
            object_id: id,
            slot: KnowledgeSlot::TableDefaultTime,
            value: ClaimPayload::default_time_column("created_at").unwrap(),
            origin: ProposalOrigin::AssistantInferred,
            confidence: 0.95,
        };

        let resolved = resolve_proposal(extracted, &table, &registry).unwrap();
        assert_eq!(resolved.profile_name, "primary");
        assert_eq!(resolved.object.qualified_name(), "analytics.public.orders");
        assert_eq!(resolved.slot, KnowledgeSlot::TableDefaultTime);
        assert_eq!(resolved.state, KnowledgeState::Pending);
        assert_eq!(resolved.source, ClaimOrigin::AssistantInferred);
        assert_eq!(
            resolved.schema_binding,
            SchemaBinding::Column {
                column: "created_at".to_string(),
                requirement: ColumnRequirement::Time,
            }
        );
    }

    #[test]
    fn test_resolve_assigns_active_for_user_and_pending_for_assistant() {
        let identity = test_identity();
        let registry = test_registry("primary", &identity);
        let mut table = TurnObjectTable::new();
        let id = table.register("primary", "public.users", &[]).unwrap();

        // User explicit -> Active
        let user_prop = ExtractedProposal {
            object_id: id.clone(),
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("user_accounts").unwrap(),
            origin: ProposalOrigin::UserExplicit,
            confidence: 1.0,
        };
        let user_resolved = resolve_proposal(user_prop, &table, &registry).unwrap();
        assert_eq!(user_resolved.state, KnowledgeState::Active);
        assert_eq!(user_resolved.source, ClaimOrigin::UserExplicit);
        assert_eq!(user_resolved.schema_binding, SchemaBinding::Table);

        // Assistant inferred -> Pending
        let asst_prop = ExtractedProposal {
            object_id: id,
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("user_accounts").unwrap(),
            origin: ProposalOrigin::AssistantInferred,
            confidence: 0.8,
        };
        let asst_resolved = resolve_proposal(asst_prop, &table, &registry).unwrap();
        assert_eq!(asst_resolved.state, KnowledgeState::Pending);
        assert_eq!(asst_resolved.source, ClaimOrigin::AssistantInferred);
    }

    #[test]
    fn test_resolve_rejects_unknown_turn_object_id() {
        let identity = test_identity();
        let registry = test_registry("primary", &identity);
        let table = TurnObjectTable::new();

        let extracted = ExtractedProposal {
            object_id: TurnObjectId::new(99),
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("orders").unwrap(),
            origin: ProposalOrigin::UserExplicit,
            confidence: 1.0,
        };

        let err = resolve_proposal(extracted, &table, &registry).unwrap_err();
        assert!(matches!(err, ResolutionError::ObjectIdNotFound(_)));
    }

    #[test]
    fn test_resolve_rejects_missing_profile_connection() {
        let identity = test_identity();
        let registry = test_registry("primary", &identity);
        let mut table = TurnObjectTable::new();
        let id = table.register("secondary", "orders", &[]).unwrap();

        let extracted = ExtractedProposal {
            object_id: id,
            slot: KnowledgeSlot::TableAlias,
            value: ClaimPayload::table_alias("orders").unwrap(),
            origin: ProposalOrigin::UserExplicit,
            confidence: 1.0,
        };

        let err = resolve_proposal(extracted, &table, &registry).unwrap_err();
        assert!(matches!(err, ResolutionError::ConnectionError(_, _)));
    }

    #[test]
    fn test_resolve_rejects_mismatched_slot_and_payload() {
        let identity = test_identity();
        let registry = test_registry("primary", &identity);
        let mut table = TurnObjectTable::new();
        let id = table.register("primary", "orders", &[]).unwrap();

        let extracted = ExtractedProposal {
            object_id: id,
            slot: KnowledgeSlot::TableDescription,
            // Value is an alias payload, not table description!
            value: ClaimPayload::table_alias("orders").unwrap(),
            origin: ProposalOrigin::UserExplicit,
            confidence: 1.0,
        };

        let err = resolve_proposal(extracted, &table, &registry).unwrap_err();
        assert_eq!(err, ResolutionError::InvalidSchemaBinding);
    }
}
