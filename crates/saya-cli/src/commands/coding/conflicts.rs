use super::{CodingApplication, CodingError};
use saya_harness::coding::integration::RetainedConflict;
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest, WorkspacePath};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};

pub(super) const MAX_LOG_BYTES: usize = 1024 * 1024;
pub(super) const MAX_RECORD_BYTES: usize = 64 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    job_id: String,
    candidate_checkout_id: String,
    status: String,
    authority: String,
    resolution: String,
    fresh_revalidation_required: bool,
    pairs: Vec<Pair>,
    inputs: Vec<Input>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    first: usize,
    second: usize,
    path: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Input {
    checkout_id: String,
    content_sha256s: Vec<Option<String>>,
    total_bytes: u64,
}

pub(super) fn retained_value(job: &JobId, conflict: &RetainedConflict) -> Value {
    json!({
        "version": 1,
        "job_id": job,
        "candidate_checkout_id": conflict.candidate_checkout_id(),
        "status": "open",
        "authority": "none",
        "resolution": "not_implemented",
        "fresh_revalidation_required": true,
        "pairs": conflict.pairs().iter().map(|pair| json!({
            "first": pair.first(), "second": pair.second(), "path": pair.path()
        })).collect::<Vec<_>>(),
        "inputs": conflict.inputs().iter().map(|input| json!({
            "checkout_id": input.checkout_id(),
            "content_sha256s": input.content_sha256s(),
            "total_bytes": input.total_bytes()
        })).collect::<Vec<_>>()
    })
}

impl CodingApplication {
    pub async fn conflicts(
        &self,
        job: &JobId,
        candidate: &ManagedCheckoutId,
    ) -> Result<Value, CodingError> {
        self.status(job).await?;
        let page = self
            .store
            .list_managed_checkouts(job, None, 100)
            .await
            .map_err(CodingError::owner)?;
        if page.next_cursor.is_some() {
            return Err(CodingError::InvalidRequest);
        }
        let ownership: BTreeMap<_, _> = page
            .items
            .iter()
            .map(|item| {
                (
                    item.ownership.managed_checkout_id().as_str().to_owned(),
                    item.ownership.agent_id().is_some(),
                )
            })
            .collect();
        if ownership.get(candidate.as_str()) != Some(&false) {
            return Err(CodingError::NotOwned);
        }
        let path = self.state_root.join(job.as_str()).join("conflicts.jsonl");
        let bytes = match super::context::read(&path, MAX_LOG_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if not_found(&error) => return Ok(unknown(job, candidate)),
            Err(error) => return Err(error),
        };
        if bytes.is_empty() || bytes.last() != Some(&b'\n') {
            return Err(CodingError::InvalidRequest);
        }
        let mut found = None;
        for line in bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            if line.len() > MAX_RECORD_BYTES {
                return Err(CodingError::InvalidRequest);
            }
            let record: Record = serde_json::from_slice(line).map_err(CodingError::owner)?;
            validate(&record, job, &ownership)?;
            if record.candidate_checkout_id == candidate.as_str() {
                found = Some(serde_json::to_value(record).map_err(CodingError::owner)?);
            }
        }
        Ok(match found {
            Some(conflict) => {
                json!({"job_id":job,"candidate_checkout_id":candidate,"status":"open","authority":"none","resolution":"not_implemented","fresh_revalidation_required":true,"conflict":conflict})
            }
            None => unknown(job, candidate),
        })
    }
}

fn unknown(job: &JobId, candidate: &ManagedCheckoutId) -> Value {
    json!({"job_id":job,"candidate_checkout_id":candidate,"status":"not_retained","authority":"none","resolution":"not_implemented","fresh_revalidation_required":true,"conflict":null})
}
fn validate(
    record: &Record,
    job: &JobId,
    ownership: &BTreeMap<String, bool>,
) -> Result<(), CodingError> {
    if record.version != 1
        || record.job_id != job.as_str()
        || record.status != "open"
        || record.authority != "none"
        || record.resolution != "not_implemented"
        || !record.fresh_revalidation_required
        || record.pairs.is_empty()
        || record.pairs.len() > 256
        || record.inputs.len() < 2
        || record.inputs.len() > 100
        || ownership.get(&record.candidate_checkout_id) != Some(&false)
    {
        return Err(CodingError::InvalidRequest);
    }
    ManagedCheckoutId::parse(&record.candidate_checkout_id).map_err(CodingError::owner)?;
    for input in &record.inputs {
        ManagedCheckoutId::parse(&input.checkout_id).map_err(CodingError::owner)?;
        if ownership.get(&input.checkout_id) != Some(&true)
            || input.total_bytes > 64 * 1024 * 1024
            || input.content_sha256s.len() > 100_000
        {
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
fn not_found(error: &CodingError) -> bool {
    let CodingError::Owner(source) = error else {
        return false;
    };
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(source.as_ref());
    while let Some(error) = current {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
        {
            return true;
        }
        current = error.source();
    }
    false
}
