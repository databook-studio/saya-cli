//! R08: graph and path admission through the public native scheduler.

use super::{
    child_support::{FactoryStats, OpenAiFactory},
    fixture::{Fixture, limits},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};
use saya_agent::CancellationToken;
use saya_harness::coding::{
    budget::JobBudgetLedger,
    coordinator::{ScheduleError, ScheduleLimits, ScheduleReport, ScheduledChild, run_schedule},
    integration::AssignedPaths,
};
use saya_types::AgentId;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};

pub(super) async fn node(
    fixture: &Fixture,
    server: &LocalOpenAi,
    name: &str,
    path: &str,
    dependencies: Vec<AgentId>,
    provider: &str,
    stats: Arc<FactoryStats>,
) -> ScheduledChild {
    let authority = fixture.add_child(name, &format!("invocation-{name}")).await;
    let spec = fixture.spec(authority, "complete assigned task", 2, 1);
    let factory = OpenAiFactory {
        endpoint: server.endpoint.clone(),
        database: fixture.database.clone(),
        job: fixture.job.clone(),
        stats,
        cancel_on_build: None,
        retry_delays: vec![],
    };
    ScheduledChild::new(
        spec,
        dependencies,
        Some(AssignedPaths::parse([path.to_owned()]).unwrap()),
        provider.to_owned(),
        Arc::new(factory),
    )
    .unwrap()
}

pub(super) fn caps() -> ScheduleLimits {
    ScheduleLimits {
        max_active_children: 3,
        max_active_per_provider: 3,
    }
}

pub(super) async fn ledger(fixture: &Fixture) -> JobBudgetLedger {
    fixture
        .ledger(limits(12, 12, 8, 4, 3, Duration::from_secs(20)))
        .await
        .unwrap()
}

pub(super) struct Running {
    pub caller: CancellationToken,
    pub task: JoinHandle<Result<ScheduleReport, ScheduleError>>,
    pub progress: JoinHandle<()>,
}

pub(super) fn start(
    nodes: Vec<ScheduledChild>,
    limits: ScheduleLimits,
    ledger: JobBudgetLedger,
) -> Running {
    let (sender, mut receiver) = mpsc::channel(128);
    let progress = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let caller = CancellationToken::new();
    let child_caller = caller.clone();
    let task =
        tokio::spawn(
            async move { run_schedule(nodes, limits, &ledger, child_caller, sender).await },
        );
    Running {
        task,
        progress,
        caller,
    }
}

pub(super) async fn finish(running: Running) -> ScheduleReport {
    let report = tokio::time::timeout(Duration::from_secs(5), running.task)
        .await
        .expect("schedule must join")
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), running.progress)
        .await
        .expect("progress owner must finish")
        .unwrap();
    report
}

#[tokio::test]
async fn scheduler_blocks_dependency_and_path_overlap_while_disjoint_child_starts() {
    let fixture = Fixture::new("r08-dag-paths").await;
    let ledger = ledger(&fixture).await;
    let mut a = LocalOpenAi::new().await;
    let mut b = LocalOpenAi::new().await;
    let mut c = LocalOpenAi::new().await;
    let mut d = LocalOpenAi::new().await;
    let b_stats = Arc::new(FactoryStats::default());
    let c_stats = Arc::new(FactoryStats::default());
    let nodes = vec![
        node(
            &fixture,
            &a,
            "agent-a",
            "src",
            vec![],
            "provider-a",
            Arc::default(),
        )
        .await,
        node(
            &fixture,
            &b,
            "agent-b",
            "tests",
            vec![AgentId::parse("agent-a").unwrap()],
            "provider-b",
            b_stats.clone(),
        )
        .await,
        node(
            &fixture,
            &c,
            "agent-c",
            "SRC/nested",
            vec![],
            "provider-c",
            c_stats.clone(),
        )
        .await,
        node(
            &fixture,
            &d,
            "agent-d",
            "docs",
            vec![],
            "provider-d",
            Arc::default(),
        )
        .await,
    ];
    let running = start(nodes, caps(), ledger.clone());
    assert_eq!(a.request().await, Some(1));
    assert_eq!(d.request().await, Some(1));
    // Three slots are available: C is blocked by its path, B by its dependency.
    assert_eq!(ledger.snapshot().await.child_slots_active, 2);
    assert_eq!(b_stats.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(c_stats.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    a.respond_sse(ANSWER_RESPONSE);
    assert_eq!(b.request().await, Some(1));
    assert_eq!(c.request().await, Some(1));
    b.respond_sse(ANSWER_RESPONSE);
    c.respond_sse(ANSWER_RESPONSE);
    d.respond_sse(ANSWER_RESPONSE);
    let report = finish(running).await;
    assert!(report.stop.is_none());
    assert!(report.not_started.is_empty());
    assert_eq!(report.outcomes.len(), 4);
    assert!(report.outcomes.iter().all(|outcome| outcome.result.is_ok()));
    assert_eq!(ledger.snapshot().await.child_slots_active, 0);
    a.shutdown().await;
    b.shutdown().await;
    c.shutdown().await;
    d.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn cycle_is_refused_before_factory_or_durable_child_reservation() {
    let fixture = Fixture::new("r08-cycle").await;
    let ledger = ledger(&fixture).await;
    let server = LocalOpenAi::new().await;
    let stats = Arc::new(FactoryStats::default());
    let nodes = vec![
        node(
            &fixture,
            &server,
            "agent-a",
            "src",
            vec![AgentId::parse("agent-b").unwrap()],
            "provider",
            stats.clone(),
        )
        .await,
        node(
            &fixture,
            &server,
            "agent-b",
            "tests",
            vec![AgentId::parse("agent-a").unwrap()],
            "provider",
            stats.clone(),
        )
        .await,
    ];
    let (sender, _receiver) = mpsc::channel(32);
    assert!(matches!(
        run_schedule(nodes, caps(), &ledger, CancellationToken::new(), sender).await,
        Err(ScheduleError::Cycle)
    ));
    assert_eq!(stats.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 100)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(ledger.cancellation_token().reason(), None);
    server.shutdown().await;
    fixture.close().await;
}
