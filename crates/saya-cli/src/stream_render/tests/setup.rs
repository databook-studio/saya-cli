use crate::render::{RenderFormat, render_event};
use saya_agent::{
    BudgetDimension, KnowledgeOutcome, LearningSkipReason, LocalStateEffect, OverrideFindingDto,
    SuppliedClaimDto, SuppliedContractDto, ToolEffect,
};
use saya_types::{ClaimId, ClaimStatus};

fn dto_claim(id: &str, kind: &str, value: &str, status: ClaimStatus) -> SuppliedClaimDto {
    SuppliedClaimDto {
        claim_id: ClaimId::parse(id).unwrap(),
        kind: kind.into(),
        value: value.into(),
        column: None,
        status,
    }
}

fn dto_contract(claims: Vec<SuppliedClaimDto>) -> SuppliedContractDto {
    SuppliedContractDto {
        profile: "analytics".into(),
        object: "catalog.public.orders".into(),
        schema_state: "current".into(),
        claims,
    }
}
