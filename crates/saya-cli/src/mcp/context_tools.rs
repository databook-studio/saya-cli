//! The `contracts` tool (task Db): approved business context over MCP. Two
//! forms — every object's active contract for the profile (the same
//! confirmed-only recall the `contracts list` command runs) and one object's
//! contract when `table` is given (the same show path). The opaque profile
//! identity never appears: both go through the single identity-dropping
//! `contract_view` mapping the CLI uses.

use rmcp::model::{CallToolRequestParams, CallToolResponse};
use saya_store::{KnowledgeItemStore as _, KnowledgeObjectsQuery, MAX_KNOWLEDGE_PAGE_SIZE};
use saya_types::{DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity};

use super::{context::McpContext, policy::ServePolicy, tools};
use crate::contracts::{
    RecallBounds, RecallMode, RecallRequest, RetrievalPolicy, now_unix_ms, recall,
    show as show_contract,
};

/// `contracts { profile, table? }` — Active claims only, no identities
/// (invariant 2). Not a row-returning tool: claims are the user's own
/// reviewed context, not query rows.
pub(crate) async fn contracts(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let name = tools::required_string(request, "profile")?;
    let table = tools::optional_string(request, "table")?;
    let (_profile, identity) = match context.allowed_profile(policy.allowlist(), name) {
        Ok(pair) => pair,
        Err(message) => return Ok(tools::error_result(message)),
    };
    match table {
        Some(table) => show_one(policy, context, name, identity, table).await,
        None => list_active(policy, context, name, identity).await,
    }
}

/// One object: the same `show` operation `contracts show` runs, classified
/// against the cached schema so a missing cache stays the honest
/// `live_schema_unavailable`, never a fabricated state.
async fn show_one(
    policy: &ServePolicy,
    context: &McpContext,
    name: &str,
    identity: ProfileIdentity,
    table: &str,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let qualified = match crate::contracts::args::parse_qualified(table) {
        Ok(qualified) => qualified,
        Err(_) => {
            return Ok(tools::error_result(
                "the table must be exactly three dot-separated parts: catalog.schema.object",
            ));
        }
    };
    let object = match DatabaseObjectRef::new(
        identity.clone(),
        &qualified.catalog,
        &qualified.schema,
        &qualified.object,
        DatabaseObjectKind::Table,
    ) {
        Ok(object) => object,
        Err(_) => {
            return Ok(tools::error_result(
                "the table must be exactly three dot-separated parts: catalog.schema.object",
            ));
        }
    };
    let cached = crate::commands::cached_schema_availability(&context.store, &identity).await;
    let retrieved = match show_contract(
        &context.store,
        &object,
        &cached,
        RetrievalPolicy::ForHumanReview,
        now_unix_ms(),
    )
    .await
    {
        Ok(retrieved) => retrieved,
        Err(error) => return Ok(tools::error_result(error.to_string())),
    };
    let payload = match retrieved {
        Some(contract) => {
            match serde_json::to_value(crate::commands::contract_view(&contract, name)) {
                Ok(payload) => payload,
                Err(_) => {
                    return Ok(tools::error_result(
                        "the contract could not be rendered as JSON",
                    ));
                }
            }
        }
        None => serde_json::json!({
            "profile": name,
            "object": object.qualified_name(),
            "claims": [],
            "note": "no contract is recorded for this object",
        }),
    };
    tools::bounded_result(policy, payload)
}

/// Every object: the same confirmed-only recall `contracts list` runs,
/// seeded with the profile's own stored objects as explicit refs, with the
/// selection, bounds, and validity rules all applied inside recall.
async fn list_active(
    policy: &ServePolicy,
    context: &McpContext,
    name: &str,
    identity: ProfileIdentity,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let explicit_refs: Vec<DatabaseObjectRef> =
        match KnowledgeObjectsQuery::first_page(MAX_KNOWLEDGE_PAGE_SIZE) {
            Err(_) => Vec::new(),
            Ok(query) => match context
                .store
                .objects_for_profile_page(&identity, query)
                .await
            {
                Ok(page) => page.entries,
                Err(_) => {
                    return Ok(tools::error_result(
                        "Local state store unavailable; contracts could not be read.",
                    ));
                }
            },
        };
    let cached = crate::commands::cached_schema_availability(&context.store, &identity).await;
    let schema_pair = (identity.clone(), cached);
    let request = RecallRequest {
        profiles: std::slice::from_ref(&identity),
        explicit_refs: &explicit_refs,
        terms: &[],
        allow_database_context: true,
        schemas: std::slice::from_ref(&schema_pair),
        now_unix_ms: now_unix_ms(),
        bounds: RecallBounds::defaults(),
        recall_mode: RecallMode::Confirmed,
        admit_candidate: None,
        policy: RetrievalPolicy::ForHumanReview,
    };
    let outcome = recall(&context.store, request).await;
    if outcome.diagnostics.store_unavailable {
        return Ok(tools::error_result(
            "Local state store unavailable; contracts could not be read.",
        ));
    }
    let contracts: Vec<_> = outcome
        .contracts
        .iter()
        .map(|contract| crate::commands::contract_view(contract, name))
        .collect();
    tools::bounded_result(policy, serde_json::json!({ "contracts": contracts }))
}
