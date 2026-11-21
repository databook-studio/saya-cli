//! The root run's plugin seam: what an active plugin adds to a root turn.
//!
//! The registry is the one door the helper surfaces already compose through
//! (`agent/tools/child/plugins.rs`): definitions are appended after the core
//! tools in registration order, a name an active plugin declares is routed to
//! that plugin's executor, and the per-turn hooks are called once each. A run
//! with no registry consults none of this and sends exactly the request bytes it
//! always did.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ActivePlugins, AgentEventSink, BackgroundExit, ContextBlock, ToolDefinition, ToolError,
    ToolExecutor, TurnOutcome,
};

/// The definitions a turn advertises: the core's own list first, then every
/// active plugin's, which is the order the helper factory composes them in.
///
/// The one `[tools] allow` list governs the whole list, the plugins' names
/// included: a list that leaves `request_clarification` out leaves it out of
/// the advertisement exactly as it leaves out the core's own governed tools.
pub(super) fn definitions(
    core: Vec<ToolDefinition>,
    plugins: &ActivePlugins,
    tools: &saya_config::ResolvedTools,
) -> Vec<ToolDefinition> {
    let mut definitions = core;
    definitions.extend(plugins.definitions());
    crate::agent::tools::narrow(&mut definitions, tools);
    definitions
}

/// The active plugins' context blocks for this turn's prompt, bounded by
/// `budget_bytes`.
pub(super) async fn context(
    plugins: &ActivePlugins,
    prompt: &str,
    budget_bytes: usize,
) -> Vec<ContextBlock> {
    plugins.context(prompt, budget_bytes).await
}

/// States the turn's end to every active plugin, once. A plugin observes the
/// outcome; it never changes it.
pub(super) async fn turn_finished(
    plugins: &ActivePlugins,
    outcome: &TurnOutcome<'_>,
    sink: &dyn AgentEventSink,
) {
    plugins.on_turn_finish(outcome, sink).await;
}

/// A root turn's executor while plugins are active: a name an active plugin
/// declares is the plugin's to serve, and every other name is the run's own
/// executor. A call that routes to a plugin is the turn's use of it, so the
/// turn's plugin budget arms there, before the call executes.
pub(super) struct Routed {
    /// The run's one `[tools] allow` list, applied to the whole advertised
    /// list after the plugins' names join it.
    tools: saya_config::ResolvedTools,
    plugins: Arc<ActivePlugins>,
    core: Arc<dyn ToolExecutor>,
    arming: super::parent_arming::TurnArming,
}

impl Routed {
    pub(super) fn new(
        plugins: Arc<ActivePlugins>,
        core: Arc<dyn ToolExecutor>,
        arming: super::parent_arming::TurnArming,
        tools: saya_config::ResolvedTools,
    ) -> Self {
        Self {
            plugins,
            core,
            arming,
            tools,
        }
    }

    /// The executor that serves `name`, or the run's own when no active plugin
    /// declares it. A plugin answers, so the turn has used it: the budget arms
    /// here, before the call it is used for executes.
    async fn owner(&self, name: &str) -> Option<Arc<dyn ToolExecutor>> {
        let owner = self.plugins.route(name);
        if owner.is_some() {
            self.arming.arm_on_route().await;
        }
        owner
    }
}

#[async_trait]
impl ToolExecutor for Routed {
    fn turn_context_note(&self) -> Option<String> {
        self.core.turn_context_note()
    }

    /// A plugin's definitions are appended to whatever the core advertises, so
    /// the list the loop re-reads before each request keeps the core's order
    /// and grows only where a plugin added a name — and the run's one
    /// `[tools] allow` list then narrows the whole list.
    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        self.core
            .advertised_definitions()
            .map(|core| definitions(core, &self.plugins, &self.tools))
    }

    fn take_background_exits(&self) -> Vec<BackgroundExit> {
        self.core.take_background_exits()
    }

    fn background_live(&self) -> bool {
        self.core.background_live()
    }

    fn background_notice_pending(&self) -> bool {
        self.core.background_notice_pending()
    }

    async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        self.core.wait_background_exit(deadline, cancellation).await
    }

    async fn settle_invocation(&self) -> Result<(), ToolError> {
        self.core.settle_invocation().await
    }

    async fn turn_finished(&self) {
        self.core.turn_finished().await;
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        match self.owner(name).await {
            Some(owner) => owner.execute_with_context(name, arguments, context).await,
            None => {
                self.core
                    .execute_with_context(name, arguments, context)
                    .await
            }
        }
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        match self.owner(name).await {
            Some(owner) => {
                owner
                    .execute_with_result_cap(name, arguments, result_cap)
                    .await
            }
            None => {
                self.core
                    .execute_with_result_cap(name, arguments, result_cap)
                    .await
            }
        }
    }

    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        match self.owner(name).await {
            Some(owner) => owner.execute(name, arguments).await,
            None => self.core.execute(name, arguments).await,
        }
    }
}
