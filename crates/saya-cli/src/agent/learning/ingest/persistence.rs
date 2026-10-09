use super::IngestionError;
use crate::agent::learning::resolver::ResolvedProposal;
use crate::agent::recall_context::claim_value;
use saya_agent::ProposedClaimDto;
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore};
use saya_types::{
    ClaimId, ClaimPayload, ClaimStatus, DatabaseObjectRef, KnowledgeSlot, KnowledgeState,
    SchemaFingerprint,
};

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
