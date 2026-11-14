//! The root's read-only surface: a job with no working checkout (no approved
//! write path, or an in-place root) reads and searches its own folder but can
//! write nothing. The workspace tools are the shared read-only family, bound to
//! the job's root; the database and ask tools ride the coordinator exactly as
//! they do on a working checkout.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ImmutableAuthorityRefusal, ToolDefinition, ToolError, ToolExecutor,
};
use serde_json::Value;

use crate::agent::tools::DatabaseTools;
use crate::commands::coding::{CodingError, NativeCodingJob};

use super::parent::CodingTools;

pub(crate) struct ReadOnlyTools {
    coordinator: CodingTools,
    workspace: DatabaseTools,
    defs: Vec<ToolDefinition>,
}

impl ReadOnlyTools {
    /// Composes the read-only surface over the job's own folder. A write is
    /// never composed: the workspace definitions are the read family only, and
    /// the coordinator's database tools are the standard read-only pair.
    pub(crate) fn build(job: &Arc<NativeCodingJob>) -> Result<Self, CodingError> {
        let workspace = DatabaseTools::for_child_checkout(job.read_only_workspace()?);
        let coordinator = CodingTools {
            job: Arc::clone(job),
            databases: job.databases(),
            denials: super::owner_denial::DenialTally::default(),
        };
        let mut defs = Vec::new();
        crate::agent::tools::workspace::append_definitions(&mut defs, false);
        if let Some(databases) = coordinator.databases.as_ref() {
            defs.extend(databases.definitions(None));
        }
        // The same evidence rule every coding surface applies: `list` is
        // covered by the search tools unless the request or `[tools] allow`
        // asks for it.
        let evidence =
            crate::agent::tools::advertisement::Evidence::new(job.task().to_owned(), false);
        defs.retain(|definition| {
            crate::agent::tools::advertisement::advertised(&evidence, job.tools(), &definition.name)
        });
        crate::agent::tools::narrow(&mut defs, job.tools());
        Ok(Self {
            coordinator,
            workspace,
            defs,
        })
    }

    pub(crate) fn definitions(&self) -> Vec<ToolDefinition> {
        self.defs.clone()
    }

    fn advertises(&self, name: &str) -> bool {
        self.defs.iter().any(|definition| definition.name == name)
    }
}

/// The canonical name a call resolves to: `bash` is the model's spelling of
/// `shell`, which this surface never serves.
fn canonical(name: &str) -> &str {
    crate::agent::tools::workspace::canonical_name(name)
}

#[async_trait]
impl ToolExecutor for ReadOnlyTools {
    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        None
    }

    async fn settle_invocation(&self) -> Result<(), ToolError> {
        self.workspace.settle_invocation().await
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        let name = canonical(name);
        if !self.advertises(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if crate::commands::coding::database::routes_here(name) {
            return self
                .coordinator
                .execute_with_context(name, arguments, context)
                .await;
        }
        self.workspace
            .execute_with_context(name, arguments, context)
            .await
    }

    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let name = canonical(name);
        if !self.advertises(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if crate::commands::coding::database::routes_here(name) {
            return self.coordinator.execute(name, arguments).await;
        }
        self.workspace.execute(name, arguments).await
    }
}

#[async_trait]
impl ApprovalDecider for ReadOnlyTools {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        if self.advertises(&tool.name) {
            return true;
        }
        // A write-shaped call is the same closed class every checkout-less
        // toolset refuses, with the same wording and repeat guard.
        self.coordinator.approve(tool, arguments).await
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        self.coordinator.refusal_detail(tool, arguments)
    }

    fn immutable_authority_refusal(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<ImmutableAuthorityRefusal> {
        self.coordinator
            .immutable_authority_refusal(tool, arguments)
    }
}
