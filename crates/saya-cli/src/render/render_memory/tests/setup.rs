use saya_agent::{
    AgentEvent, KnowledgeOutcome, OverrideFindingDto, SuppliedClaimDto, SuppliedContractDto,
};
use saya_types::{ClaimId, ClaimStatus};

fn claim(
    id: &str,
    kind: &str,
    value: &str,
    column: Option<&str>,
    status: ClaimStatus,
) -> SuppliedClaimDto {
    SuppliedClaimDto {
        claim_id: ClaimId::parse(id).unwrap(),
        kind: kind.into(),
        value: value.into(),
        column: column.map(str::to_string),
        status,
    }
}

fn contract(
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

/// `KnowledgeOutcome::Ran { store_unavailable }` is a struct variant;
/// a local constructor reads more clearly at the call sites than the brace
/// form repeated in every fixture.
fn ran(store_unavailable: bool) -> KnowledgeOutcome {
    KnowledgeOutcome::Ran { store_unavailable }
}
