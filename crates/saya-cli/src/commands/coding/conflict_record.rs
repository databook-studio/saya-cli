use super::CodingError;
use saya_harness::coding::integration::{EvidenceBinding, RetainedConflict};
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest, WorkspacePath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) const MAX_RECORD_BYTES: usize = 64 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub(super) version: u8,
    pub(super) job_id: String,
    pub(super) candidate_checkout_id: String,
    pub(super) status: String,
    pub(super) authority: String,
    pub(super) resolution: String,
    pub(super) fresh_revalidation_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) candidate_source_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) record_sha256: Option<String>,
    pub(super) pairs: Vec<Pair>,
    pub(super) inputs: Vec<Input>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pair {
    pub(super) first: usize,
    pub(super) second: usize,
    pub(super) path: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) checkout_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) source_sha256: Option<String>,
    pub(super) content_sha256s: Vec<Option<String>>,
    pub(super) total_bytes: u64,
}

pub(super) fn retained_bytes(
    job: &JobId,
    conflict: &RetainedConflict,
    candidate_binding: &EvidenceBinding,
    source_bindings: &[EvidenceBinding],
) -> Result<Vec<u8>, CodingError> {
    if candidate_binding.candidate_checkout_id() != conflict.candidate_checkout_id() {
        return Err(CodingError::NotOwned);
    }
    let inputs = conflict
        .inputs()
        .iter()
        .map(|input| {
            let binding = source_bindings
                .iter()
                .find(|binding| binding.candidate_checkout_id() == input.checkout_id())
                .ok_or(CodingError::NotOwned)?;
            Ok(Input {
                checkout_id: input.checkout_id().to_string(),
                source_sha256: Some(binding.source_sha256().to_string()),
                content_sha256s: input
                    .content_sha256s()
                    .iter()
                    .map(|digest| digest.as_ref().map(ToString::to_string))
                    .collect(),
                total_bytes: input.total_bytes(),
            })
        })
        .collect::<Result<Vec<_>, CodingError>>()?;
    let mut record = Record {
        version: 2,
        job_id: job.to_string(),
        candidate_checkout_id: conflict.candidate_checkout_id().to_string(),
        status: "open".into(),
        authority: "none".into(),
        resolution: "not_implemented".into(),
        fresh_revalidation_required: true,
        candidate_source_sha256: Some(candidate_binding.source_sha256().to_string()),
        record_sha256: None,
        pairs: conflict
            .pairs()
            .iter()
            .map(|pair| Pair {
                first: pair.first(),
                second: pair.second(),
                path: pair.path().into(),
            })
            .collect(),
        inputs,
    };
    // Struct field order and vector order define the canonical bytes; the
    // self-referential record_sha256 field is omitted while hashing.
    let canonical = serde_json::to_vec(&record).map_err(CodingError::owner)?;
    record.record_sha256 = Some(super::create::digest(&canonical)?.to_string());
    serde_json::to_vec(&record).map_err(CodingError::owner)
}

pub(super) fn validate_retained_bytes(bytes: &[u8], job: &JobId) -> Result<(), CodingError> {
    parse_retained_record(bytes, job).map(|_| ())
}

pub(super) fn parse_retained_record(bytes: &[u8], job: &JobId) -> Result<Record, CodingError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(CodingError::InvalidRequest);
    }
    let record: Record = serde_json::from_slice(bytes).map_err(CodingError::owner)?;
    validate_record(&record, job)?;
    Ok(record)
}

fn validate_record(record: &Record, job: &JobId) -> Result<(), CodingError> {
    if !(record.version == 1 || record.version == 2)
        || record.job_id != job.as_str()
        || record.status != "open"
        || record.authority != "none"
        || record.resolution != "not_implemented"
        || !record.fresh_revalidation_required
        || record.pairs.is_empty()
        || record.inputs.len() < 2
        || record.inputs.len() > 100
    {
        return Err(CodingError::InvalidRequest);
    }
    if record.version == 1 {
        if record.candidate_source_sha256.is_some()
            || record.record_sha256.is_some()
            || record
                .inputs
                .iter()
                .any(|input| input.source_sha256.is_some())
        {
            return Err(CodingError::InvalidRequest);
        }
    } else {
        Sha256Digest::parse(
            record
                .candidate_source_sha256
                .as_deref()
                .ok_or(CodingError::InvalidRequest)?,
        )
        .map_err(CodingError::owner)?;
        for input in &record.inputs {
            Sha256Digest::parse(
                input
                    .source_sha256
                    .as_deref()
                    .ok_or(CodingError::InvalidRequest)?,
            )
            .map_err(CodingError::owner)?;
        }
        let declared = Sha256Digest::parse(
            record
                .record_sha256
                .as_deref()
                .ok_or(CodingError::InvalidRequest)?,
        )
        .map_err(CodingError::owner)?;
        let mut canonical_record = record.clone();
        canonical_record.record_sha256 = None;
        let canonical = serde_json::to_vec(&canonical_record).map_err(CodingError::owner)?;
        if super::create::digest(&canonical)? != declared {
            return Err(CodingError::InvalidRequest);
        }
    }
    ManagedCheckoutId::parse(&record.candidate_checkout_id).map_err(CodingError::owner)?;
    for input in &record.inputs {
        ManagedCheckoutId::parse(&input.checkout_id).map_err(CodingError::owner)?;
        if input.total_bytes > 64 * 1024 * 1024 || input.content_sha256s.len() > 100_000 {
            return Err(CodingError::InvalidRequest);
        }
        for digest in input.content_sha256s.iter().flatten() {
            Sha256Digest::parse(digest).map_err(CodingError::owner)?;
        }
    }
    for pair in &record.pairs {
        if pair.first >= record.inputs.len()
            || pair.second >= record.inputs.len()
            || pair.first == pair.second
            || pair.path.len() > 512
            || WorkspacePath::try_from(pair.path.clone()).is_err()
        {
            return Err(CodingError::InvalidRequest);
        }
    }
    Ok(())
}

pub(super) fn validate_ownership(
    record: &Record,
    ownership: &BTreeMap<String, bool>,
) -> Result<(), CodingError> {
    if ownership.get(&record.candidate_checkout_id) != Some(&false)
        || record
            .inputs
            .iter()
            .any(|input| ownership.get(&input.checkout_id) != Some(&true))
    {
        return Err(CodingError::InvalidRequest);
    }
    Ok(())
}
