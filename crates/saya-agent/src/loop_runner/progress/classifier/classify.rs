//! What one settled call is: a change, a wait, or nothing. The shell, search
//! and read families each answer against the semantic key and the result, so
//! a successful call that returned nothing new is not progress.

use serde_json::Value;

use super::super::ToolObservation;
use super::super::classifier_keys::{has_matches, has_output, output_digest, read_key, shell_key};
use super::super::inspection;
use super::super::{SILENT_POLL_LIMIT, SILENT_WAIT_TOTAL};
use super::{Classification, ProgressClassifier};

impl ProgressClassifier {
    pub(super) fn event_kind(&self, observation: ToolObservation<'_>) -> &'static str {
        match observation.name {
            "write" | "edit" | "notebook_edit" | "delete" | "rewind" => "workspace_change",
            "shell" | "run_command" | "shell_output" | "shell_kill" => "outcome_change",
            "read" | "grep" | "glob" | "list" => "new_read",
            "checklist" => "checklist",
            "submit_plan" => "plan",
            _ => "other",
        }
    }

    pub(super) fn classify(
        &mut self,
        observation: ToolObservation<'_>,
        result_digest: u64,
    ) -> Classification {
        if !observation.succeeded {
            return Classification::Idle;
        }
        match observation.name {
            // A successful write family call changed bytes.
            "write" | "edit" | "notebook_edit" | "delete" | "rewind" => Classification::Progress,
            "shell" | "run_command" => self.classify_shell(observation, result_digest),
            // A poll of a background run: new output or a new status is
            // progress; a silent wait on a live run is neither progress nor
            // a stuck turn.
            "shell_output" | "shell_kill" => self.classify_background(observation),
            // A new read target is progress; the same range returning changed
            // bytes is progress too; the same range with the same result is
            // not.
            "read" | "list" => {
                let key = read_key(observation.name, observation.arguments);
                let changed = self
                    .read_results
                    .insert(key, result_digest)
                    .is_none_or(|previous| previous != result_digest);
                if changed {
                    Classification::Progress
                } else {
                    Classification::Idle
                }
            }
            // A search is progress only when it found something new: a new
            // pattern that matches nothing moved nothing.
            "grep" | "glob" => self.classify_search(observation, result_digest),
            // A checklist whose statuses changed.
            "checklist" => {
                let changed = self.last_checklist_digest != Some(result_digest);
                self.last_checklist_digest = Some(result_digest);
                if changed {
                    Classification::Progress
                } else {
                    Classification::Idle
                }
            }
            // A submitted plan.
            "submit_plan" => Classification::Progress,
            _ => Classification::Idle,
        }
    }

    /// A shell call counts only when it moved something. A read-only
    /// inspection command (`cat`, `grep`, `find`, `ls`, `sed -n`, ...) is
    /// classified as the read tool it stands in for; a non-read command that
    /// succeeded silently changed the workspace (`mkdir`, `chmod`, `mv`,
    /// `git add`/`commit`, an install) and is a neutral step, not churn; every
    /// other command counts when its exit code or normalized output changed
    /// and the result carried something (or failed).
    fn classify_shell(
        &mut self,
        observation: ToolObservation<'_>,
        result_digest: u64,
    ) -> Classification {
        let exit = exit_code(observation.result);
        if let Some(command) = observation.arguments.get("command").and_then(Value::as_str) {
            if let Some(inspection) = inspection::parse(command) {
                let material = exit == 0 && has_output(observation.result);
                let changed = self
                    .inspection_results
                    .insert(inspection.key(), result_digest)
                    .is_none_or(|previous| previous != result_digest);
                return if material && changed {
                    Classification::Progress
                } else {
                    Classification::Idle
                };
            }
            // Only reads, searches and inspection repeats build the idle
            // streak. A silent success from any other command is a workspace
            // change we cannot see, so it neither advances nor resets it.
            if exit == 0 && !has_output(observation.result) {
                return Classification::Neutral;
            }
        }
        let key = shell_key(observation.name, observation.arguments);
        let digest = output_digest(observation.result);
        let material = exit != 0 || has_output(observation.result);
        let changed = self
            .shell_outcomes
            .insert(key, (exit, digest))
            .is_none_or(|previous| previous != (exit, digest));
        if material && changed {
            Classification::Progress
        } else {
            Classification::Idle
        }
    }

    /// A search is progress when it found a match and the result differs from
    /// the last call with the same key.
    fn classify_search(
        &mut self,
        observation: ToolObservation<'_>,
        result_digest: u64,
    ) -> Classification {
        let key = read_key(observation.name, observation.arguments);
        let changed = self
            .search_results
            .insert(key, result_digest)
            .is_none_or(|previous| previous != result_digest);
        if has_matches(observation.result) && changed {
            Classification::Progress
        } else {
            Classification::Idle
        }
    }

    /// A background read's contribution. The output is the bytes written
    /// since the last read, so any text is new; a status the run has not
    /// reported before is a change too. A silent poll of a still-running run
    /// is the wait itself: neutral, not a stuck turn and not a repeat.
    fn classify_background(&mut self, observation: ToolObservation<'_>) -> Classification {
        let Some(id) = observation.arguments.get("id").and_then(Value::as_u64) else {
            return Classification::Idle;
        };
        let status = observation
            .result
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let new_output = ["stdout", "stderr"].iter().any(|field| {
            observation
                .result
                .get(*field)
                .and_then(Value::as_str)
                .is_some_and(|text| !text.is_empty())
        });
        let changed_status = match self.background_status.insert(id, status.to_owned()) {
            Some(previous) => previous != status,
            // A first look at a run that has already ended is a status
            // change; a first look at a live one is just the wait.
            None => status != "running",
        };
        if new_output || changed_status {
            self.silent_polls.remove(&id);
            Classification::Progress
        } else if status == "running" {
            let polls = {
                let polls = self.silent_polls.entry(id).or_insert(0);
                *polls = polls.saturating_add(1);
                *polls
            };
            // A silent poll of a live run is a wait, not churn. Past the cap
            // it stays neutral while the run may still be working, up to the
            // job's wall clock (or the fixed total wait bound when the job has
            // none); only past that bound is it an idle turn the guards can
            // see.
            if polls <= SILENT_POLL_LIMIT || self.silent_wait_available() {
                Classification::Neutral
            } else {
                Classification::Idle
            }
        } else {
            self.silent_polls.remove(&id);
            Classification::Idle
        }
    }

    /// Whether a silent poll of a live run may still count as a wait. The
    /// job's wall clock bounds it; a job with no wall clock gets the fixed
    /// total wait bound, so a run that never speaks cannot hold the guards off
    /// forever.
    fn silent_wait_available(&self) -> bool {
        match self.remaining_wall() {
            Some(remaining) => !remaining.is_zero(),
            None => self.started.elapsed() < SILENT_WAIT_TOTAL,
        }
    }
}

/// The exit code a shell result reported; a result without one did not fail.
fn exit_code(result: &Value) -> i64 {
    result.get("exit_code").and_then(Value::as_i64).unwrap_or(0)
}
