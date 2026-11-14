use crate::commands::coding::NativeCodingJob;
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition, ToolError, ToolExecutor};
use serde_json::Value;
use std::sync::Arc;

pub(crate) struct CodingTools {
    pub(crate) job: Arc<NativeCodingJob>,
    pub(crate) databases: Option<crate::commands::coding::database::CodingDatabaseTools>,
    /// The read-only job's denial tally: this toolset owns no write path, so
    /// every write it is asked about is refused with the same closed class.
    pub(crate) denials: super::owner_denial::DenialTally,
}
/// The database tools the workspace owns, when the job has a profile. The
/// coordinator's own delegation surface is `helpers`, advertised separately by
/// `DirectCodingTools`; no supervisor tools exist.
pub(crate) fn definitions(
    databases: Option<&crate::commands::coding::database::CodingDatabaseTools>,
) -> Vec<ToolDefinition> {
    let mut tools = Vec::new();
    if let Some(databases) = databases {
        tools.extend(databases.definitions(None));
    }
    tools
}
#[async_trait]
impl ToolExecutor for CodingTools {
    async fn execute_with_context(
        &self,
        name: &str,
        args: Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        if crate::commands::coding::database::routes_here(name) {
            let databases = self.databases.as_ref().ok_or(ToolError::UnsupportedTool)?;
            return databases.execute(None, name, args, context).await;
        }
        self.execute(name, args).await
    }
    async fn execute(&self, name: &str, args: Value) -> Result<Value, ToolError> {
        // The ask needs no context and no connection, so this door serves it
        // directly — through the same advertised gate the context door applies.
        if crate::agent::tools::clarification::is_ask(name) {
            let databases = self.databases.as_ref().ok_or(ToolError::UnsupportedTool)?;
            return databases.ask_result().ok_or(ToolError::UnsupportedTool);
        }
        let result = match name {
            "helpers" => super::task_tool::execute(&self.job, args).await,
            _ => return Err(ToolError::UnsupportedTool),
        };
        result.map_err(tool_failure)
    }
}
#[async_trait]
impl ApprovalDecider for CodingTools {
    async fn approve(&self, tool: &ToolDefinition, _: &Value) -> bool {
        // This owner exists only after fresh job approval. Exact path/program
        // attenuation is enforced by `helpers`; children never receive this toolset.
        let approved = definitions(self.databases.as_ref())
            .iter()
            .any(|expected| expected.name == tool.name && expected.effect == tool.effect);
        if !approved && let Some(denial) = self.write_denial(tool) {
            self.denials.note(denial.class());
        }
        approved
    }

    /// This toolset carries no checkout, so it approves no write path: every
    /// workspace write, edit or delete it is asked about is the same closed
    /// class, worded so the model can delegate instead of retrying.
    fn refusal_detail(&self, tool: &ToolDefinition, _: &Value) -> Option<String> {
        let denial = self.write_denial(tool)?;
        Some(self.denials.wording(&denial))
    }
}

impl CodingTools {
    fn write_denial(&self, tool: &ToolDefinition) -> Option<super::owner_denial::OwnerWriteDenial> {
        crate::agent::tools::workspace::is_write_tool(&tool.name)
            .then_some(super::owner_denial::OwnerWriteDenial::NoWritePath)
    }
}
fn error(message: &str) -> ToolError {
    ToolError::Runner(message.into())
}

pub(super) fn refusal(message: &str) -> ToolError {
    ToolError::NoEffect(Box::new(error(message)))
}

pub(super) fn coding_refusal(error: crate::commands::coding::CodingError) -> ToolError {
    refusal(&error.to_string())
}

fn tool_failure(error: crate::commands::coding::CodingError) -> ToolError {
    // A refusal raised before any child is registered, any checkout taken, any
    // history line appended or any ledger event written proves the call had no
    // effect. `Busy` refuses before the call starts; `AssignmentTooLong`,
    // `UnknownSubagentType` and `InvalidRequest` are raised by `helpers`
    // admission — `one_helper` and `validate_delegate_tasks` — before
    // `delegate` stages anything. The composition failure that can happen after
    // registration is re-classified in `commands::coding::delegate::prepare_child`,
    // so it never reads as an `InvalidRequest` here.
    if matches!(
        error,
        crate::commands::coding::CodingError::Busy
            | crate::commands::coding::CodingError::AssignmentTooLong
            | crate::commands::coding::CodingError::UnknownSubagentType { .. }
            | crate::commands::coding::CodingError::InvalidRequest
    ) {
        coding_refusal(error)
    } else {
        ToolError::Runner(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal raised before any child is registered, any checkout taken,
    /// any history line appended or any ledger event written proves the call
    /// had no effect, so its reservation settles known.
    #[test]
    fn refusals_before_any_effect_settle_known() {
        use crate::commands::coding::CodingError;
        for refusal in [
            CodingError::Busy,
            CodingError::AssignmentTooLong,
            CodingError::UnknownSubagentType {
                requested: "reviewer".into(),
                known: "explore, general".into(),
            },
            CodingError::InvalidRequest,
        ] {
            let rendered = refusal.to_string();
            assert!(
                tool_failure(refusal).proves_no_effect(),
                "{rendered} is refused before the call has any effect"
            );
        }
    }

    /// A failure past the pre-staging window — an ownership check, an approval
    /// gate, or a composition failure after registration — is not proof of no
    /// effect, so its reservation stays uncertain.
    #[test]
    fn failures_after_staging_stay_uncertain() {
        use crate::commands::coding::CodingError;
        for failure in [
            CodingError::NotOwned,
            CodingError::ApprovalRequired,
            CodingError::owner(std::io::Error::other("composition failed")),
        ] {
            let rendered = failure.to_string();
            assert!(
                !tool_failure(failure).proves_no_effect(),
                "a failure past the pre-staging window ({rendered}) is not proof of no effect"
            );
        }
    }
}
