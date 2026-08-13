//! Mapping `crate::contracts::view` results to the agent tool's JSON payload.
//!
//! Reuses `crate::commands::contracts::contract_view` — the single identity-
//! dropping `RetrievedContract → ContractView` mapping — so the opaque
//! `ProfileIdentity` never appears in a tool result (the DTO has no field for
//! it). The agent sees the profile *name* the registry resolved, never the
//! identity. See .claude/specs/spec-2b3a-agent-contract-tools.md §2.
//!
//! Also owns the empty-result shapes and the short reasons they carry: a tool
//! that returns nothing tells the model *why*, so it does not retry the same
//! unproductive call.

use crate::commands::contract_view;
use crate::contracts::RetrievedContract;
use crate::render::ContractView;

/// Maximum claims a `contract_read` result carries. The `show` operation does
/// not bound claims, so the tool layer applies the 2b-1 per-object default
/// itself and flags truncation — see the SPEC REVIEW (Defect 2) for 2b-3a.
const MAX_READ_CLAIMS: usize = 12;

/// Short, stable reason strings for empty results.
pub(super) const REASON_PRIVACY: &str = "database context is disabled for this provider";
pub(super) const REASON_NO_IDENTITY: &str =
    "no profile identity is associated with this connection";
pub(super) const REASON_STORE: &str = "local contract store is unavailable";
pub(super) const REASON_NO_MATCH: &str = "no contract matches the given terms";
pub(super) const REASON_NO_CONTRACT: &str = "no contract is stored for this object";

/// Serializes one object's contract for the model. The `ContractView` DTO has
/// no field for the opaque identity, so neither does this payload.
fn to_json(view: &ContractView) -> serde_json::Value {
    serde_json::to_value(view).expect("ContractView serializes to JSON")
}

/// One object's contract, as the model sees it. `contract_search` uses this
/// directly; recall has already bounded and flagged truncation.
pub(super) fn contract_payload(
    contract: &RetrievedContract,
    profile_name: &str,
) -> serde_json::Value {
    to_json(&contract_view(contract, profile_name))
}

/// Like [`contract_payload`] but caps the claims at `MAX_READ_CLAIMS`, flagging
/// `truncated` when the object had more. Used by `contract_read`, where the
/// underlying `show` operation is unbounded.
pub(super) fn read_payload(contract: &RetrievedContract, profile_name: &str) -> serde_json::Value {
    let mut view = contract_view(contract, profile_name);
    if view.claims.len() > MAX_READ_CLAIMS {
        view.claims.truncate(MAX_READ_CLAIMS);
        view.truncated = true;
    }
    to_json(&view)
}

/// Selects the empty-result shape for a tool: search results carry a `contracts`
/// array (empty when nothing matched); read results carry only a `reason`, so the
/// model never mistakes a missing `contract` for a present-but-empty one.
pub(super) fn empty_for(name: &str) -> fn(&str) -> serde_json::Value {
    match name {
        "contract_read" => empty_read,
        _ => empty_search,
    }
}

/// A non-empty `contract_search` result.
pub(super) fn contracts(contracts: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "contracts": contracts })
}

/// A non-empty `contract_read` result.
pub(super) fn contract(payload: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "contract": payload })
}

fn empty_search(reason: &str) -> serde_json::Value {
    serde_json::json!({ "contracts": [], "reason": reason })
}

fn empty_read(reason: &str) -> serde_json::Value {
    serde_json::json!({ "reason": reason })
}
