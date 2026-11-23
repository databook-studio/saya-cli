//! The executor that answers `use_database`: the one tool a run offers before
//! its database family is latched.
//!
//! It wraps the surface's own executor rather than replacing it, so every other
//! call — the database family's included, once it is on — takes exactly the
//! route it took before this tool existed.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolDefinition, ToolError, ToolExecutionContext, ToolExecutor};

use super::family::{Family, USE_DATABASE};

pub(super) struct UseDatabase {
    inner: Arc<dyn ToolExecutor>,
    family: Arc<Family>,
}

impl UseDatabase {
    pub(super) fn new(inner: Arc<dyn ToolExecutor>, family: Arc<Family>) -> Self {
        Self { inner, family }
    }

    /// Answers the ask: latches the family, so the next request carries the
    /// database tools, and states the connections it is now on for.
    fn answer(&self) -> serde_json::Value {
        self.family.latch();
        self.family.result()
    }
}

#[async_trait]
impl ToolExecutor for UseDatabase {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if name == USE_DATABASE {
            return Ok(self.answer());
        }
        self.inner.execute(name, arguments).await
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        if name == USE_DATABASE {
            return Ok(self.answer());
        }
        self.inner
            .execute_with_result_cap(name, arguments, result_cap)
            .await
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        if name == USE_DATABASE {
            return Ok(self.answer());
        }
        self.inner
            .execute_with_context(name, arguments, context)
            .await
    }

    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        self.inner.advertised_definitions()
    }
}
