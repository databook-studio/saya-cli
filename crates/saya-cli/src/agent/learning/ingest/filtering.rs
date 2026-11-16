use crate::agent::learning::resolver::ResolvedProposal;
use crate::agent::learning::turn_record::SuppliedContractDto;
use crate::agent::recall_context::claim_value;
use crate::contracts::SuppliedContract;
use saya_types::ClaimOrigin;

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
