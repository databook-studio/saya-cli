//! Read-only contract tools for the agent: `contract_search` and `contract_read`.
//!
//! Both call the 2b-1 operations ([`recall`] and [`show`]) and add no policy of
//! their own. The opaque profile identity never appears in a tool result — the
//! model sees the profile *name*. Bounds are the 2b-1 defaults; a truncated
//! result says so. Only recallable claims are returned. Store failure degrades
//! to an empty result with a diagnostic, never a tool error. See
//! .claude/specs/spec-2b3a-agent-contract-tools.md §2.

mod definitions;
mod mapping;
mod validation;

pub(crate) use definitions::definitions as contract_tool_definitions;

use saya_agent::ToolError;
use saya_types::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};

use mapping::{
    REASON_NO_CONTRACT, REASON_NO_IDENTITY, REASON_NO_MATCH, REASON_PRIVACY, REASON_STORE,
    contract, contract_payload, contracts, empty_for, read_payload,
};
use validation::validate_arguments;

use super::DatabaseTools;
use crate::contracts::args::parse_qualified;
use crate::contracts::{RecallBounds, RecallMode, RecallRequest, recall, show as show_contract};
use saya_store::SchemaStore;

impl DatabaseTools {
    /// Dispatches a contract tool call: validates arguments, resolves the
    /// connection to a profile identity, applies the privacy gate, and calls
    /// the 2b-1 operation. Store failure and a closed gate return an empty
    /// result with a reason, never `Err`.
    pub(super) async fn execute_contract_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        validate_arguments(name, &arguments)?;
        let connection = arguments
            .get("connection")
            .and_then(serde_json::Value::as_str);
        let entry = self.registry.resolve(connection)?;
        let profile_name = resolve_name(self.registry.primary_name(), connection);
        let empty = empty_for(name);
        let Some(identity) = entry.profile_id.as_deref() else {
            return Ok(empty(REASON_NO_IDENTITY));
        };
        let identity =
            ProfileIdentity::parse(identity).map_err(|_| ToolError::InvalidQueryArguments)?;

        if !self.allow_query_data {
            return Ok(empty(REASON_PRIVACY));
        }
        let Some(store) = self.state_db.as_ref() else {
            return Ok(empty(REASON_STORE));
        };

        match name {
            "contract_search" => {
                self.contract_search(store, &identity, &profile_name, &arguments, empty)
                    .await
            }
            "contract_read" => {
                self.contract_read(store, &identity, &profile_name, &arguments, empty)
                    .await
            }
            _ => Err(ToolError::UnsupportedTool),
        }
    }

    async fn contract_search(
        &self,
        store: &saya_store::SqliteStateStore,
        identity: &ProfileIdentity,
        profile_name: &str,
        arguments: &serde_json::Value,
        empty: fn(&str) -> serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let terms: Vec<String> = arguments
            .get("terms")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let request = RecallRequest {
            profiles: std::slice::from_ref(identity),
            explicit_refs: &[],
            terms: &terms,
            allow_database_context: true,
            schemas: &[],
            bounds: RecallBounds::defaults(),
            // The agent's own search tool stays Confirmed-only: a candidate is
            // not an established fact, and surfacing one through a read tool the
            // model trusts would let inference read inference as confirmation.
            // Candidates reach the model only through the context block (recall
            // mode), where the render layer labels them unconfirmed.
            recall_mode: RecallMode::Confirmed,
        };
        // Recall degrades a store failure to an empty outcome with a diagnostic;
        // surface the diagnostic as the reason so the model does not retry.
        let outcome = recall(store, request).await;
        if outcome.diagnostics.store_unavailable {
            return Ok(empty(REASON_STORE));
        }
        if outcome.contracts.is_empty() {
            return Ok(empty(REASON_NO_MATCH));
        }
        let payload: Vec<serde_json::Value> = outcome
            .contracts
            .iter()
            .map(|c| contract_payload(c, profile_name))
            .collect();
        Ok(contracts(payload))
    }

    async fn contract_read(
        &self,
        store: &saya_store::SqliteStateStore,
        identity: &ProfileIdentity,
        profile_name: &str,
        arguments: &serde_json::Value,
        empty: fn(&str) -> serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let table = arguments
            .get("table")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::InvalidQueryArguments)?;
        let qualified = parse_qualified(table).map_err(|_| ToolError::InvalidQueryArguments)?;
        let object = DatabaseObjectRef::new(
            identity.clone(),
            &qualified.catalog,
            &qualified.schema,
            &qualified.object,
            DatabaseObjectKind::Table,
        )
        .map_err(|_| ToolError::InvalidQueryArguments)?;

        // The cached schema for the profile classifies the claim, so the model
        // sees `current`/`needs_review`/`stale` — the same projection the CLI's
        // `show` renders — not a constant `live_schema_unavailable`. A missing
        // cache stays `None` (the honest answer), mirroring the CLI adapter.
        let schema = store
            .get_schema(identity.as_str())
            .await
            .ok()
            .flatten()
            .map(|cached| cached.schema);
        match show_contract(store, &object, schema.as_ref()).await {
            Ok(Some(retrieved)) => Ok(contract(read_payload(&retrieved, profile_name))),
            Ok(None) => Ok(empty(REASON_NO_CONTRACT)),
            Err(_) => Ok(empty(REASON_STORE)),
        }
    }
}

/// The connection name the model passed, or the primary when it did not. This is
/// the profile *name* the result carries — never the opaque identity.
fn resolve_name(primary: &str, connection: Option<&str>) -> String {
    match connection {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => primary.to_string(),
    }
}
