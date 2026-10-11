use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use saya_agent::{
    AllowReadOnlyApproval, LocalStateEffect, OpenAiCompatibleProvider, ProviderSettings,
    ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
};
use saya_harness::coding::child::{
    ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError,
};
use saya_store::SqliteStateStore;
use saya_types::{BudgetDimension, JobId};

#[derive(Default)]
pub(super) struct FactoryStats {
    pub builds: AtomicUsize,
    pub executor_calls: AtomicUsize,
    pub executor_saw_tool_reservation: AtomicBool,
    pub invocation_cancellation: Mutex<Option<saya_agent::CancellationToken>>,
}

impl FactoryStats {
    pub(super) fn assert_invocation_cancellation(&self, expected: saya_agent::CancellationReason) {
        let actual = self
            .invocation_cancellation
            .lock()
            .unwrap()
            .as_ref()
            .and_then(saya_agent::CancellationToken::reason);
        assert_eq!(actual, Some(expected));
    }
}

pub(super) struct OpenAiFactory {
    pub endpoint: String,
    pub database: std::path::PathBuf,
    pub job: JobId,
    pub stats: Arc<FactoryStats>,
    pub cancel_on_build: Option<saya_agent::CancellationToken>,
    pub retry_delays: Vec<Duration>,
}

impl ChildFactory for OpenAiFactory {
    fn build(
        &self,
        _spec: &ChildExecutionSpec,
        cancellation: &saya_agent::CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError> {
        self.stats.builds.fetch_add(1, Ordering::SeqCst);
        *self.stats.invocation_cancellation.lock().unwrap() = Some(cancellation.clone());
        if let Some(cancellation) = &self.cancel_on_build {
            cancellation.cancel_with_reason(saya_agent::CancellationReason::User);
        }
        let provider = OpenAiCompatibleProvider::new(
            ProviderSettings::new("r03a-test-model", Some(self.endpoint.clone()))
                .with_timeout(Duration::from_secs(3))
                .with_idle_timeout(Duration::from_secs(3))
                .with_retry_delays(self.retry_delays.clone()),
            None,
        )
        .map_err(ChildFactoryError::new)?;
        Ok(ChildComponents::new(
            Box::new(provider),
            Box::new(CountingExecutor {
                database: self.database.clone(),
                job: self.job.clone(),
                stats: Arc::clone(&self.stats),
            }),
            vec![tool_definition()],
            Box::new(AllowReadOnlyApproval),
        ))
    }
}

pub(super) fn tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "inspect".to_owned(),
        description: "Inspect assigned metadata".to_owned(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Concurrent,
        completion: Some("inspection complete".to_owned()),
    }
}

struct CountingExecutor {
    database: std::path::PathBuf,
    job: JobId,
    stats: Arc<FactoryStats>,
}

#[async_trait]
impl ToolExecutor for CountingExecutor {
    async fn execute(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.stats.executor_calls.fetch_add(1, Ordering::SeqCst);
        let store = SqliteStateStore::new(self.database.clone());
        let page = store
            .list_coding_budget_reservations(&self.job, None, 100)
            .await;
        let saw_tool_charge = page.is_ok_and(|page| {
            page.items.iter().any(|record| {
                record
                    .reservation
                    .charges()
                    .iter()
                    .any(|charge| charge.dimension() == BudgetDimension::ToolCalls)
            })
        });
        self.stats
            .executor_saw_tool_reservation
            .store(saw_tool_charge, Ordering::SeqCst);
        Ok(serde_json::json!({"ok":true}))
    }
}
