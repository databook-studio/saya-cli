use super::{
    CodingApplication, CodingError,
    conflict_record::{self, parse_retained_record, validate_ownership},
};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io};

pub(super) const MAX_LOG_BYTES: usize = 1024 * 1024;
pub(super) use conflict_record::{retained_bytes, validate_retained_bytes};

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
            let record = parse_retained_record(line, job)?;
            validate_ownership(&record, &ownership)?;
            if record.candidate_checkout_id == candidate.as_str() {
                let record_sha256 = record
                    .record_sha256
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::String(super::create::digest(line)?.to_string()));
                let provenance = json!({
                    "complete": record.version == 2,
                    "source_sha256s_complete": record.version == 2,
                    "candidate_source_sha256": record.candidate_source_sha256.clone(),
                    "record_sha256_scope": if record.version == 2 { "canonical_record_without_record_sha256" } else { "raw_json_line" },
                    "record_sha256": record_sha256
                });
                found = Some((
                    serde_json::to_value(record).map_err(CodingError::owner)?,
                    provenance,
                ));
            }
        }
        Ok(match found {
            Some((conflict, provenance)) => {
                json!({"job_id":job,"candidate_checkout_id":candidate,"status":"retained","record_status_at_capture":"open","current_state":"unknown_requires_fresh_revalidation","authority":"none","resolution":"not_implemented","fresh_revalidation_required":true,"manual_edit":manual_edit(job,candidate),"provenance":provenance,"conflict":conflict})
            }
            None => unknown(job, candidate),
        })
    }
}

fn manual_edit(job: &JobId, candidate: &ManagedCheckoutId) -> Value {
    let review_command = format!("coding review-existing {job} {candidate} --approve");
    let apply_command = format!(
        "coding apply {job} {candidate} --source-sha256 SOURCE_SHA256_FROM_REVIEW --approve"
    );
    json!({
        "candidate_checkout_id":candidate,
        "next_action":"run_review_existing_to_locate_candidate_edit_then_rerun_fresh_review",
        "locate_candidate":{"command":review_command,"path_field":"candidate_path"},
        "edit_scope":"only_the_returned_candidate_path",
        "review_command":review_command,
        "apply_command":apply_command,
        "fresh_review":{"owned_checkout_verification":true,"configured_checks":true,"source_digest_binding":true},
        "apply_command_note":"replace SOURCE_SHA256_FROM_REVIEW with the exact source_sha256 returned by the second review-existing response",
        "retained_record_mutated":false,
        "applies_to_original":false,
        "later_apply_requires_fresh_approval":true,
        "later_apply_requires_valid_receipts":true
    })
}

fn unknown(job: &JobId, candidate: &ManagedCheckoutId) -> Value {
    json!({"job_id":job,"candidate_checkout_id":candidate,"status":"not_retained","authority":"none","resolution":"not_implemented","fresh_revalidation_required":true,"conflict":null})
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
