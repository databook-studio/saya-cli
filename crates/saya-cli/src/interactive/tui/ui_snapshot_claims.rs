/// Claim, render, and wide-table fixtures (split from the shared fixtures).
use saya_agent::{ProposedClaimDto, SuppliedClaimDto, SuppliedContractDto};
use saya_types::{ClaimStatus, QueryResult};

use super::super::table::format_table;
use super::super::transcript::BlockKind;
use super::super::types::App;
use super::support::empty_app;

/// A stable, fixed claim id. `abbreviate_id` keeps the first six chars + `…`
/// (len > 7), so `ki-abcdef1234` renders as `ki-abc…` — deterministic for fixed
/// input (spec: a `ki-…` prefix is fine when stable).
pub(crate) fn claim_id(id: &str) -> saya_types::ClaimId {
    saya_types::ClaimId::parse(id).expect("fixed claim id parses")
}

pub(crate) fn supplied_claim(
    id: &str,
    kind: &str,
    value: &str,
    column: Option<&str>,
    status: ClaimStatus,
) -> SuppliedClaimDto {
    SuppliedClaimDto {
        claim_id: claim_id(id),
        kind: kind.into(),
        value: value.into(),
        column: column.map(str::to_string),
        status,
    }
}

pub(crate) fn supplied_contract(
    profile: &str,
    object: &str,
    state: &str,
    claims: Vec<SuppliedClaimDto>,
) -> SuppliedContractDto {
    SuppliedContractDto {
        profile: profile.into(),
        object: object.into(),
        schema_state: state.into(),
        claims,
    }
}

pub(crate) fn proposed_claim(
    id: &str,
    profile: &str,
    object: &str,
    kind: &str,
    value: &str,
    column: Option<&str>,
    status: ClaimStatus,
) -> ProposedClaimDto {
    ProposedClaimDto {
        claim_id: claim_id(id),
        profile: profile.into(),
        object: object.into(),
        kind: kind.into(),
        value: value.into(),
        column: column.map(str::to_string),
        status,
    }
}
/// A 12-column result that overflows the text area at 80×24, so the view must
/// scroll horizontally rather than word-wrap the grid into noise.
pub(crate) fn wide_table_result() -> QueryResult {
    QueryResult {
        columns: vec![
            "id".into(),
            "name".into(),
            "status".into(),
            "region".into(),
            "country".into(),
            "city".into(),
            "postal".into(),
            "carrier".into(),
            "tracking".into(),
            "total".into(),
            "tax".into(),
            "shipped_at".into(),
        ],
        rows: vec![serde_json::json!([
            1,
            "alice",
            "fulfilled",
            "north",
            "CA",
            "SF",
            "94105",
            "UPS",
            "1Z999",
            120,
            12,
            "2024-01-03"
        ])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT * FROM orders".into(),
    }
}

pub(crate) fn app_with_wide_table() -> App {
    let mut app = empty_app();
    // A direct /sql command lands as a user line, then the result table — the
    // table alone would not count as a "turn", so the user line is needed for
    // the transcript pane (not the splash) to render.
    app.transcript
        .push(BlockKind::User, "/sql SELECT * FROM orders");
    app.transcript
        .push(BlockKind::Table, format_table(&wide_table_result()));
    app
}
