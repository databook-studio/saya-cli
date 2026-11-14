//! Export only durable owner records and explicitly named completeness limits.
use super::{CodingApplication, CodingError, context};
use saya_harness::coding::{
    git::{GitOperation, GitProcessEffect, GitProcessOutcome, GitProcessSnapshot},
    worktree::{InPlaceRootInput, verify_in_place_root},
};
use saya_types::JobId;
use serde_json::{Value, json};
use std::{num::NonZeroU64, path::Path};
impl CodingApplication {
    pub async fn export(
        &self,
        job: &JobId,
        after: Option<NonZeroU64>,
        transport_after: Option<NonZeroU64>,
    ) -> Result<Value, CodingError> {
        let status = self.status(job).await?;
        let usage = status.get("usage").cloned().unwrap_or(Value::Null);
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
        let budget_mode = policy.as_ref().map(|record| record.policy.budget_mode());
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
        let local_git_processes = local_git_processes(&self.service.process_observations());
        let transports = self
            .store
            .list_coding_transport_attempts(job, transport_after, 100)
            .await
            .map_err(CodingError::owner)?;
        let transport_complete = transports.next_cursor.is_none()
            && transports.items.iter().all(|item| item.timing_complete());
        let spans=transports.items.iter().map(|item|json!({"reservation_sequence":item.reservation.reservation_sequence(),"reservation":item.reservation,"span":item.span,"settlement":item.settlement,"timing_complete":item.timing_complete()})).collect::<Vec<_>>();
        let mut value = json!({"schema_version":1,"job_id":job,"repository_id":record.repository_id,"snapshot_id":record.snapshot_id,
            "snapshot_sha256":snapshot.captured_manifest_sha256(),"plan_sha256":snapshot.plan_manifest_sha256(),"status":status,"usage":usage,
            "budget_mode":budget_mode,
            "events":{"source":"durable_coding_owner","items":events.events,"next_cursor":events.next_cursor,"page_complete":events.next_cursor.is_none(),"after":after},
            "budget_policy":policy.map(|record|json!({"sha256":record.policy_sha256,"limits":record.policy})),
            "requests_and_usage":{"source":"durable_budget_events","complete_after_all_event_pages":true,"unsettled_is_unknown":true},
            "transport_spans":{"complete":transport_complete,"items":spans,"next_cursor":transports.next_cursor,"after":transport_after,"scope":"actual_client_http_future_first_poll_to_finish_not_remote_server_execution"},
            "local_git_processes":local_git_processes,
            "processes":{"source":"check_command_extension_and_local_git_owners","complete":false,"reason":"extension/check process coverage, job attribution, remote/global processes, platform proof, and uncertain spans remain incomplete"},"token_accounting":{"complete":false,"mode":"count_only_no_verified_provider_input_bound","stored_policy":token_policy.map(|policy|json!({"sha256":policy.digest(),"data":policy.data()}))},
            "artifacts":artifacts,"answers_are_evidence":false,"stored_data_grants_authority":false});
        // In-place facts are additive and stated only for an in-place job, so
        // the Git export keeps its exact published shape.
        if self.is_in_place() {
            let baseline = self.stored_baseline(job, &record)?;
            if let Some(baseline) = baseline.in_place() {
                let records = self.state_root.join(job.as_str());
                let root = super::root_checkout::InPlaceRootRecord::read(
                    &records.join("inplace-root.json"),
                )?;
                let recorded = verify_in_place_root(&InPlaceRootInput {
                    artifacts: &self.artifacts,
                    state_root: &self.state_root,
                    root_id: root.root_id(),
                    root_path: Path::new(root.root_path()),
                    snapshot_id: root.snapshot_id(),
                    plan_sha256: root.plan_sha256(),
                })
                .ok()
                .and_then(|verified| super::in_place_changes::live(&verified, &records).ok());
                let (files, bytes, overflowed) = recorded.as_ref().map_or((0, 0, false), |live| {
                    (
                        live.changes.len() as u64,
                        live.changes
                            .iter()
                            .fold(0u64, |total, change| total.saturating_add(change.bytes)),
                        live.overflowed,
                    )
                });
                value["mode"] = json!("in_place");
                value["root"] = json!({
                    "path": baseline.root_path(),
                    "root_id": baseline.repository_id(),
                    "git": root.git(),
                });
                value["baseline"] = json!({
                    "snapshot_id": baseline.snapshot_id(),
                    "kind": "edit_time",
                });
                value["recorded"] = json!({
                    "files": files,
                    "bytes": bytes,
                    "overflowed": overflowed,
                });
            }
        }
        Ok(value)
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
                && (["provider-bindings.jsonl", "continuation.jsonl"].contains(&name.as_str())
                    || [
                        "checks-",
                        "commands-",
                        "review-",
                        "extensions-",
                        "known-commits-",
                    ]
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
            let items = if name.starts_with("known-commits-") {
                super::known_commit::export_records(&bytes)?
            } else {
                bytes
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(serde_json::from_slice::<Value>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(CodingError::owner)?
            };
            records
                .push(json!({"name":name,"sha256":super::create::digest(&bytes)?,"records":items}));
        }
        let diagnostics = match super::diagnostics::export(&root) {
            Ok(diagnostics) => diagnostics,
            Err(fault) => {
                json!({"history":{"schema_version":1,"state":"unknown"},"fault":fault,"files":[]})
            }
        };
        let complete = diagnostics["history"]["state"] == "complete";
        Ok(
            json!({"source":"bounded_private_owner_append_records","files":records,
            "diagnostics":diagnostics,"complete":complete}),
        )
    }
}

fn local_git_processes(snapshot: &GitProcessSnapshot) -> Value {
    json!({"source":"local_git_process_observer","scope":"application_service","coverage":"local_git_subprocesses_only","job_attribution":"unknown_for_pre_job_and_shared_service_calls","remote_processes_observed":false,"complete":snapshot.complete(),"omitted":snapshot.omitted(),"omitted_bytes":snapshot.omitted_bytes(),"service_identity":snapshot.service_identity(),"records":snapshot.receipts().iter().map(|receipt|json!({"schema_version":receipt.version(),"service_identity":receipt.service_identity(),"sequence":receipt.sequence(),"request_kind":receipt.request_kind(),"operation":operation(receipt.operation()),"pid":receipt.pid(),"exit_code":receipt.exit_code(),"signal":receipt.signal(),"outcome":outcome(receipt.outcome()),"raw_process_effect":effect(receipt.effect()),"orphan_sweep":receipt.killed_orphans(),"duration_ms":receipt.duration_ms(),"stdout_bytes":receipt.stdout_bytes(),"stderr_bytes":receipt.stderr_bytes(),"stdout_truncated":receipt.stdout_truncated(),"stderr_truncated":receipt.stderr_truncated()})).collect::<Vec<_>>()})
}

fn operation(value: GitOperation) -> &'static str {
    match value {
        GitOperation::Read => "read",
        GitOperation::Materialize => "materialize",
        GitOperation::Remove => "remove",
        // `#[non_exhaustive]`: a future variant is labelled unknown, mirroring
        // `outcome`/`effect` in this file.
        _ => "unknown",
    }
}
fn outcome(value: GitProcessOutcome) -> &'static str {
    match value {
        GitProcessOutcome::Exited => "exited",
        GitProcessOutcome::TimedOut => "timed_out",
        GitProcessOutcome::Cancelled => "cancelled",
        GitProcessOutcome::SpawnFailed => "spawn_failed",
        GitProcessOutcome::WaitFailed => "wait_failed",
        GitProcessOutcome::CaptureFailed => "capture_failed",
        GitProcessOutcome::OutputLimited => "output_limited",
        GitProcessOutcome::Unknown => "unknown",
    }
}
fn effect(value: GitProcessEffect) -> &'static str {
    match value {
        GitProcessEffect::None => "none",
        GitProcessEffect::ProcessSucceeded => "process_succeeded",
        GitProcessEffect::Unknown => "unknown",
    }
}
