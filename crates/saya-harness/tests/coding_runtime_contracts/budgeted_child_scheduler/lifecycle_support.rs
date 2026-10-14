use super::{child_support::tool_definition, fixture::Fixture, openai_server::LocalOpenAi};
use async_trait::async_trait;
use saya_agent::{
    AllowReadOnlyApproval, CancellationToken, OpenAiCompatibleProvider, ProviderSettings,
    ToolError, ToolExecutionContext, ToolExecutor,
};
use saya_harness::coding::{
    budget::JobBudgetLedger,
    child::{ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError},
    coordinator::ScheduledChild,
    integration::AssignedPaths,
};
#[cfg(unix)]
use saya_harness::coding::{
    budget::{BudgetedCommandSession, start_budgeted},
    commands::CommandOwner,
};
#[cfg(unix)]
use std::{num::NonZeroU64, path::PathBuf};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

#[cfg(unix)]
#[path = "../command_owner_lifecycle/support.rs"]
pub(super) mod process;

pub(super) enum Behavior {
    Panic,
    IgnoreCancellation,
    SettlementFailure,
    #[cfg(unix)]
    Command {
        live: &'static process::Fixture,
        parent: PathBuf,
        child: PathBuf,
        retain: bool,
    },
}
struct Factory {
    endpoint: String,
    #[cfg(unix)]
    ledger: JobBudgetLedger,
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
                #[cfg(unix)]
                ledger: self.ledger.clone(),
                #[cfg(unix)]
                sessions: tokio::sync::Mutex::new(Vec::new()),
                behavior: self.behavior.clone(),
                entered: self.entered.clone(),
            }),
            vec![tool_definition()],
            Box::new(AllowReadOnlyApproval),
        ))
    }
}
struct Executor {
    #[cfg(unix)]
    ledger: JobBudgetLedger,
    #[cfg(unix)]
    sessions: tokio::sync::Mutex<Vec<(CommandOwner, BudgetedCommandSession)>>,
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
        #[cfg(unix)]
        for (_owner, session) in std::mem::take(&mut *self.sessions.lock().await) {
            let joined = session.stop_and_join().await;
            assert!(joined.command.is_ok());
            assert!(joined.settlement.is_ok());
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
        #[cfg(not(unix))]
        let _ = context;
        self.entered.notify_one();
        match self.behavior.as_ref() {
            Behavior::Panic => panic!("intentional fixture tool panic after admission"),
            Behavior::IgnoreCancellation => std::future::pending().await,
            Behavior::SettlementFailure => Ok(serde_json::json!({"done":true})),
            #[cfg(unix)]
            Behavior::Command {
                live,
                parent,
                child,
                retain,
            } => {
                let owner = CommandOwner::new(1).unwrap();
                let session = start_budgeted(
                    &self.ledger,
                    &owner,
                    live.run_program().with_cancellation(context.cancellation),
                    process::HELPER.into(),
                    vec![
                        "fork-hold".into(),
                        parent.display().to_string(),
                        child.display().to_string(),
                    ],
                    NonZeroU64::new(20).unwrap(),
                )
                .await
                .unwrap();
                if *retain {
                    self.sessions.lock().await.push((owner, session));
                    return Ok(serde_json::json!({"started":true}));
                }
                let joined = session.join().await;
                assert!(joined.settlement.is_ok());
                let outcome = joined.command.unwrap();
                assert!(outcome.cancelled);
                Ok(serde_json::json!({"cancelled": true}))
            }
        }
    }
}
pub(super) async fn node(
    fixture: &Fixture,
    ledger: &JobBudgetLedger,
    server: &LocalOpenAi,
    name: &str,
    behavior: Behavior,
) -> (ScheduledChild, Arc<Notify>) {
    #[cfg(not(unix))]
    let _ = ledger;
    let authority = fixture.add_child(name, &format!("invocation-{name}")).await;
    let spec = fixture.spec(authority, "inspect assigned work", 3, 2);
    let entered = Arc::new(Notify::new());
    let node = ScheduledChild::new(
        spec,
        vec![],
        Some(AssignedPaths::parse([name.to_owned()]).unwrap()),
        name.into(),
        Arc::new(Factory {
            endpoint: server.endpoint.clone(),
            #[cfg(unix)]
            ledger: ledger.clone(),
            behavior: Arc::new(behavior),
            entered: entered.clone(),
        }),
    )
    .unwrap();
    (node, entered)
}
