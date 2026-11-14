use super::*;
use crate::commands::coding::{KnownWorkspaceMutation, KnownWorkspaceMutationOperation};
use saya_types::CodingFailureClass;

impl ChildTools {
    pub(super) async fn after_workspace_mutation(
        &self,
        name: &str,
        mut result: serde_json::Value,
        cancellation: &saya_agent::CancellationToken,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        let Some(coding) = self.coding.as_ref() else {
            return Ok(result);
        };
        let operation = match name {
            "write" => KnownWorkspaceMutationOperation::Write,
            "edit" | "notebook_edit" => KnownWorkspaceMutationOperation::Edit,
            _ => return Ok(result),
        };
        let path = result.get("path").and_then(serde_json::Value::as_str);
        let digest = result.get("digest").and_then(serde_json::Value::as_str);
        let size = result.get("size").and_then(serde_json::Value::as_u64);
        let committed = match (path, digest, size) {
            (Some(path), Some(digest), Some(size)) => {
                KnownWorkspaceMutation::new(operation, path.to_owned(), digest.to_owned(), size)
            }
            _ => {
                self.tools.read_witnesses.clear();
                coding.fail_owner_operation(CodingFailureClass::Internal, None, cancellation);
                let facts = saya_agent::shape_tool_result(&result, 1024).text;
                return Err(ToolError::Runner(format!(
                    "workspace mutation committed but committed-byte facts were unavailable: {facts}"
                )));
            }
        };
        let committed = match committed {
            Ok(committed) => committed,
            Err(error) => {
                self.tools.read_witnesses.clear();
                coding.fail_owner_operation(CodingFailureClass::Internal, None, cancellation);
                let facts = saya_agent::shape_tool_result(&result, 1024).text;
                return Err(ToolError::Runner(format!(
                    "workspace mutation committed but committed-byte facts were invalid ({error}): {facts}"
                )));
            }
        };
        if coding.journals_committed_mutations() && coding.record_known_commit(&committed).is_err()
        {
            self.tools.read_witnesses.clear();
            coding.fail_owner_operation(
                CodingFailureClass::MandatoryPersistence,
                None,
                cancellation,
            );
            return Err(ToolError::Runner(
                "workspace mutation committed but its known-commit receipt could not be written"
                    .into(),
            ));
        }
        // The notebook edit is journaled like any other committed write; it has
        // no document diagnostics of its own.
        if name == "notebook_edit" {
            return Ok(result);
        }
        match coding
            .post_edit_diagnostics(committed.clone(), cancellation)
            .await
        {
            Ok(None) => Ok(result),
            Ok(Some(feedback)) => {
                match post_edit_feedback_status(&feedback, &committed) {
                    Some("complete") => {}
                    Some("stale" | "unavailable") => {
                        self.tools
                            .read_witnesses
                            .invalidate(committed.path().as_str());
                    }
                    _ => {
                        self.tools
                            .read_witnesses
                            .invalidate(committed.path().as_str());
                        coding.fail_owner_operation(
                            CodingFailureClass::Internal,
                            None,
                            cancellation,
                        );
                        return Err(ToolError::Runner(
                            "workspace mutation committed but post-edit diagnostics returned invalid facts"
                                .into(),
                        ));
                    }
                }
                let Some(bounded) = bounded_post_edit_feedback(&result, feedback, result_cap)
                else {
                    self.tools
                        .read_witnesses
                        .invalidate(committed.path().as_str());
                    return Err(committed_result_cap_error(&committed, result_cap));
                };
                result["post_edit_diagnostics"] = bounded;
                Ok(result)
            }
            Err(error) => {
                let committed = error.committed();
                self.tools
                    .read_witnesses
                    .invalidate(committed.path().as_str());
                let failure = error.owner_failure();
                let operation = match committed.operation() {
                    KnownWorkspaceMutationOperation::Write => "write",
                    KnownWorkspaceMutationOperation::Edit => "edit",
                    KnownWorkspaceMutationOperation::Delete => "delete",
                };
                Err(ToolError::Runner(format!(
                    "workspace {operation} committed at {} with sha256 {} and size {}; post-edit diagnostics failed ({}, {})",
                    committed.path().as_str(),
                    committed.sha256(),
                    committed.size(),
                    failure.primary().as_str(),
                    failure
                        .secondary()
                        .map_or("no secondary cause", |class| class.as_str())
                )))
            }
        }
    }
}

fn post_edit_feedback_status(
    feedback: &serde_json::Value,
    committed: &KnownWorkspaceMutation,
) -> Option<&'static str> {
    let status = feedback.get("status").and_then(serde_json::Value::as_str)?;
    if feedback
        .get("document_sha256")
        .and_then(serde_json::Value::as_str)
        != Some(committed.sha256().as_str())
        || feedback.get("version").and_then(serde_json::Value::as_u64) != Some(1)
        || feedback.get("edits_applied") != Some(&serde_json::Value::Bool(false))
    {
        return None;
    }
    match status {
        "complete"
            if feedback
                .get("diagnostics")
                .and_then(serde_json::Value::as_array)
                .is_some() =>
        {
            Some("complete")
        }
        "stale" => Some("stale"),
        "unavailable" => Some("unavailable"),
        _ => None,
    }
}

fn bounded_post_edit_feedback(
    result: &serde_json::Value,
    feedback: serde_json::Value,
    result_cap: usize,
) -> Option<serde_json::Value> {
    let mut candidate = result.clone();
    candidate["post_edit_diagnostics"] = feedback.clone();
    if !saya_agent::shape_tool_result(&candidate, result_cap).truncated {
        return Some(feedback);
    }

    if let Some(diagnostics) = feedback
        .get("diagnostics")
        .and_then(serde_json::Value::as_array)
    {
        for retained in (0..=diagnostics.len()).rev() {
            let mut bounded = feedback.clone();
            bounded["diagnostics"] = serde_json::Value::Array(diagnostics[..retained].to_vec());
            bounded["diagnostics_truncated"] = serde_json::Value::Bool(true);
            candidate["post_edit_diagnostics"] = bounded.clone();
            if !saya_agent::shape_tool_result(&candidate, result_cap).truncated {
                return Some(bounded);
            }
        }
    }
    let minimum = serde_json::json!({
        "status":feedback.get("status"),
        "path":feedback.get("path"),
        "document_sha256":feedback.get("document_sha256"),
        "version":feedback.get("version"),
        "edits_applied":false,
        "diagnostics_truncated":true
    });
    candidate["post_edit_diagnostics"] = minimum.clone();
    (!saya_agent::shape_tool_result(&candidate, result_cap).truncated).then_some(minimum)
}

fn committed_result_cap_error(committed: &KnownWorkspaceMutation, result_cap: usize) -> ToolError {
    let detail = format!(
        "workspace mutation committed at {} with sha256 {} and size {}; post-edit feedback omitted because its minimum truthful result exceeds the caller cap",
        committed.path().as_str(),
        committed.sha256(),
        committed.size()
    );
    let compact = "workspace mutation committed; caller cap cannot hold committed facts; consult the known-commit receipt";
    for message in [detail.as_str(), compact, "workspace mutation committed"] {
        if !saya_agent::shape_tool_result(&serde_json::json!({"error":message}), result_cap)
            .truncated
        {
            return ToolError::Runner(message.to_owned());
        }
    }
    ToolError::Runner("workspace mutation committed".into())
}

#[cfg(test)]
#[path = "post_edit_tests.rs"]
mod tests;
