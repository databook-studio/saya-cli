//! Export only durable owner records and explicitly named completeness limits.
use super::{CodingApplication, CodingError, context};
use saya_types::JobId;
use serde_json::{Value, json};
use std::num::NonZeroU64;
impl CodingApplication {
    pub async fn export(
        &self,
        job: &JobId,
        after: Option<NonZeroU64>,
        transport_after: Option<NonZeroU64>,
    ) -> Result<Value, CodingError> {
        let status = self.status(job).await?;
        let events = self
            .store
            .list_coding_events(job, after, 100)
            .await
            .map_err(CodingError::owner)?;
        let policy = self
            .store
            .get_coding_budget_policy(job)
            .await
            .map_err(CodingError::owner)?;
        let record = self
            .store
            .get_coding_job(job)
            .await
            .map_err(CodingError::owner)?
            .ok_or(CodingError::NotOwned)?;
        let snapshot = self
            .artifacts
            .read_snapshot(&record.snapshot_id)
            .map_err(CodingError::owner)?;
        let token_policy = self
            .store
            .get_coding_token_policy(job)
            .await
            .map_err(CodingError::owner)?;
        let artifacts = self.export_artifacts(job)?;
        let transports = self
            .store
            .list_coding_transport_attempts(job, transport_after, 100)
            .await
            .map_err(CodingError::owner)?;
        let transport_complete = transports.next_cursor.is_none()
            && transports.items.iter().all(|item| item.timing_complete());
        let spans=transports.items.iter().map(|item|json!({"reservation_sequence":item.reservation.reservation_sequence(),"reservation":item.reservation,"span":item.span,"settlement":item.settlement,"timing_complete":item.timing_complete()})).collect::<Vec<_>>();
        Ok(
            json!({"schema_version":1,"job_id":job,"repository_id":record.repository_id,"snapshot_id":record.snapshot_id,
            "snapshot_sha256":snapshot.captured_manifest_sha256(),"plan_sha256":snapshot.plan_manifest_sha256(),"status":status,
            "events":{"source":"durable_coding_owner","items":events.events,"next_cursor":events.next_cursor,"page_complete":events.next_cursor.is_none(),"after":after},
            "budget_policy":policy.map(|record|json!({"sha256":record.policy_sha256,"limits":record.policy})),
            "requests_and_usage":{"source":"durable_budget_events","complete_after_all_event_pages":true,"unsettled_is_unknown":true},
            "transport_spans":{"complete":transport_complete,"items":spans,"next_cursor":transports.next_cursor,"after":transport_after,"scope":"actual_client_http_future_first_poll_to_finish_not_remote_server_execution"},
            "processes":{"source":"check_command_and_extension_owner_records","complete":false,"reason":"Git source observer binding not yet present; unmatched or uncertain extension spans remain unknown"},"token_accounting":{"complete":false,"mode":"count_only_no_verified_provider_input_bound","stored_policy":token_policy.map(|policy|json!({"sha256":policy.digest(),"data":policy.data()}))},
            "artifacts":artifacts,"answers_are_evidence":false,"stored_data_grants_authority":false}),
        )
    }
    fn export_artifacts(&self, job: &JobId) -> Result<Value, CodingError> {
        let root = self.state_root.join(job.as_str());
        let mut paths = Vec::new();
        for (index, entry) in std::fs::read_dir(&root)
            .map_err(CodingError::owner)?
            .enumerate()
            .take(1025)
        {
            let entry = entry.map_err(CodingError::owner)?;
            if index == 1024 {
                return Err(CodingError::InvalidRequest);
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| CodingError::InvalidRequest)?;
            if name.ends_with(".jsonl")
                && ([
                    "provider-bindings.jsonl",
                    "compactions.jsonl",
                    "continuation.jsonl",
                    "observations.jsonl",
                ]
                .contains(&name.as_str())
                    || ["checks-", "commands-", "review-", "extensions-"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix)))
            {
                paths.push((name, entry.path()));
            }
        }
        paths.sort_by(|left, right| left.0.cmp(&right.0));
        let mut total = 0usize;
        let mut records = Vec::new();
        for (name, path) in paths {
            let bytes = context::read(&path, 1024 * 1024)?;
            total = total
                .checked_add(bytes.len())
                .ok_or(CodingError::InvalidRequest)?;
            if total > 4 * 1024 * 1024 {
                return Err(CodingError::InvalidRequest);
            }
            let items = bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(serde_json::from_slice::<Value>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(CodingError::owner)?;
            records
                .push(json!({"name":name,"sha256":super::create::digest(&bytes)?,"records":items}));
        }
        Ok(json!({"source":"bounded_private_owner_append_records","files":records,"complete":true}))
    }
}
