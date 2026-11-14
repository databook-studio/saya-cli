use super::{child_support::tool_definition, fixture::Fixture, openai_server::LocalOpenAi};
use async_trait::async_trait;
use saya_agent::{
    AllowReadOnlyApproval, CancellationToken, OpenAiCompatibleProvider, ProviderSettings,
    ToolError, ToolExecutionContext, ToolExecutor,
};
use saya_harness::coding::{
    child::{ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError},
    coordinator::ScheduledChild,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

pub(super) enum Behavior {
    Panic,
    IgnoreCancellation,
    SettlementFailure,
}
struct Factory {
    endpoint: String,
    behavior: Arc<Behavior>,
    entered: Arc<Notify>,
}
impl ChildFactory for Factory {
    fn build(
        &self,
        _: &ChildExecutionSpec,
        _: &CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError> {
        let provider = OpenAiCompatibleProvider::new(
            ProviderSettings::new("fixture", Some(self.endpoint.clone()))
                .with_timeout(Duration::from_secs(10))
                .with_retry_delays(vec![]),
            None,
        )
        .map_err(ChildFactoryError::new)?;
        Ok(ChildComponents::new(
            Box::new(provider),
            Box::new(Executor {
                behavior: self.behavior.clone(),
                entered: self.entered.clone(),
            }),
            vec![tool_definition()],
            Box::new(AllowReadOnlyApproval),
        ))
    }
}
struct Executor {
    behavior: Arc<Behavior>,
    entered: Arc<Notify>,
}
#[async_trait]
impl ToolExecutor for Executor {
    async fn settle_invocation(&self) -> Result<(), ToolError> {
        if matches!(self.behavior.as_ref(), Behavior::SettlementFailure) {
            return Err(ToolError::Workspace(
                "fixture could not prove resource settlement".into(),
            ));
        }
        Ok(())
    }

    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        unreachable!("context-aware path")
    }
    async fn execute_with_context(
        &self,
        _: &str,
        _: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        let _ = context;
        self.entered.notify_one();
        match self.behavior.as_ref() {
            Behavior::Panic => panic!("intentional fixture tool panic after admission"),
            Behavior::IgnoreCancellation => std::future::pending().await,
            Behavior::SettlementFailure => Ok(serde_json::json!({"done":true})),
        }
    }
}
pub(super) async fn node(
    fixture: &Fixture,
    server: &LocalOpenAi,
    name: &str,
    behavior: Behavior,
) -> (ScheduledChild, Arc<Notify>) {
    let authority = fixture.add_child(name, &format!("invocation-{name}")).await;
    let spec = fixture.spec(authority, "inspect assigned work", 3, 2);
    let entered = Arc::new(Notify::new());
    let node = ScheduledChild::new(
        spec,
        vec![],
        name.into(),
        Arc::new(Factory {
            endpoint: server.endpoint.clone(),
            behavior: Arc::new(behavior),
            entered: entered.clone(),
        }),
    )
    .unwrap();
    (node, entered)
}
