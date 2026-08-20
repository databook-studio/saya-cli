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
    REASON_NO_CONTRACT, REASON_NO_IDENTITY, REASON_NO_MATCH, REASON_PRIVACY, REASON_STALE,
    REASON_STORE, contract, contract_payload, contracts, empty_for, read_payload,
};
use validation::validate_arguments;

use super::DatabaseTools;
use crate::commands::cached_schema_availability;
use crate::contracts::args::parse_qualified;
use crate::contracts::{
    RecallBounds, RecallMode, RecallRequest, RetrievalPolicy, now_unix_ms, recall,
    show as show_contract,
};

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
        // Load the cached schema the way the CLI's read commands do, so the
        // contract's staleness is *computed* — passing no schema and calling
        // the result "not stale" is the same bug in a different hat. `Missing`
        // and `Unavailable` stay distinct (the honest `live_schema_unavailable`),
        // never fabricated into a tree that could read `current`.
        let cached = cached_schema_availability(store, identity).await;
        let schema_pair = (identity.clone(), cached);
        let schemas = std::slice::from_ref(&schema_pair);
        let request = RecallRequest {
            profiles: std::slice::from_ref(identity),
            explicit_refs: &[],
            terms: &terms,
            allow_database_context: true,
            schemas,
            // The model path bounds cached-schema age: a stale-by-age cache
            // cannot vouch for currency and classifies `live_schema_unavailable`.
            now_unix_ms: now_unix_ms(),
            bounds: RecallBounds::defaults(),
            // The agent's own search tool stays Confirmed-only: a candidate is
            // not an established fact, and surfacing one through a read tool the
            // model trusts would let inference read inference as confirmation.
            // Candidates reach the model only through the context block (recall
            // mode), where the render layer labels them unconfirmed.
            recall_mode: RecallMode::Confirmed,
            // The search tool does not honour a per-claim admission; `None`
            // keeps the Confirmed-only mode.
            admit_candidate: None,
            // A model-facing path: a contract computed `Stale` is dropped (and
            // counted) so a gone-column claim never reads as a current fact.
            policy: RetrievalPolicy::ForModel,
        };
        // Recall degrades a store failure to an empty outcome with a diagnostic;
        // surface the diagnostic as the reason so the model does not retry.
        let outcome = recall(store, request).await;
        if outcome.diagnostics.store_unavailable {
            return Ok(empty(REASON_STORE));
        }
        if outcome.contracts.is_empty() {
            // Distinguish "nothing matched" from "matched but every match was
            // stale": a stale exclusion is non-silent, so the model does not
            // retry the same terms expecting a different answer.
            if outcome.diagnostics.excluded_by_schema > 0 {
                return Ok(empty(REASON_STALE));
            }
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
        // `show` renders — not a constant `live_schema_unavailable`. `Missing`
        // and `Unavailable` stay distinct (the honest answer), mirroring the
        // CLI adapter; the model path bounds the cache's age.
        let schema = cached_schema_availability(store, identity).await;
        // `contract_read` is model-facing: `show` with `ForModel` returns a
        // stale object with `schema_state: Stale` and **no claims**, so the
        // model learns the object is stale without reading a gone-column claim
        // as a current fact. The human `contracts show` path keeps the claims.
        match show_contract(
            store,
            &object,
            &schema,
            RetrievalPolicy::ForModel,
            now_unix_ms(),
        )
        .await
        {
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
