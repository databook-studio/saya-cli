use super::*;
use saya_store::{NewCodingChild, NewCodingJob};
use saya_types::{
    AgentId, BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation,
    BudgetObservationValue, BudgetReservationV1, BudgetSettlementV1, CodingEventKind,
    CodingEventV1, InvocationId, JobId, JobState, RepositoryId, SnapshotId,
};
use std::num::NonZeroU64;

mod checkout;
pub(super) use checkout::create_completed_checkout;

pub(super) struct CodingSeed {
    pub root: std::path::PathBuf,
    pub database: std::path::PathBuf,
    pub store: SqliteStateStore,
    pub job_id: JobId,
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub child: saya_store::CodingChildRecord,
    pub reservation: BudgetReservationV1,
    pub settlement: BudgetSettlementV1,
    pub events: Vec<CodingEventV1>,
}

impl CodingSeed {
    pub async fn new(label: &str) -> Self {
        let root = temp_root(label);
        let database = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&database);
        let job_id = JobId::parse("job-v12-checkout").unwrap();
        let repository_id = RepositoryId::parse("repository-v12-checkout").unwrap();
        let snapshot_id = SnapshotId::parse("snapshot-v12-checkout").unwrap();
        store
            .create_coding_job(NewCodingJob {
                job_id: job_id.clone(),
                repository_id: repository_id.clone(),
                snapshot_id: snapshot_id.clone(),
            })
            .await
            .unwrap();
        let child = store
            .create_coding_child(NewCodingChild {
                job_id: job_id.clone(),
                agent_id: AgentId::parse("agent-v12-checkout").unwrap(),
                current_invocation_id: InvocationId::parse("invocation-v12-checkout").unwrap(),
            })
            .await
            .unwrap();
        let running = event(
            1,
            &job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        );
        store.append_coding_event(running.clone()).await.unwrap();
        let reservation = BudgetReservationV1::new(
            job_id.clone(),
            NonZeroU64::new(2).unwrap(),
            Some(child.agent_id.clone()),
            Some(child.current_invocation_id.clone()),
            vec![
                BudgetCharge::new(
                    BudgetDimension::ChildSlots,
                    None,
                    None,
                    NonZeroU64::new(1).unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let reserved = event(
            2,
            &job_id,
            Some((
                child.agent_id.clone(),
                Some(child.current_invocation_id.clone()),
            )),
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation.clone()),
            },
        );
        store.reserve_coding_budget(reserved.clone()).await.unwrap();
        let settlement = BudgetSettlementV1::new(
            job_id.clone(),
            NonZeroU64::new(2).unwrap(),
            NonZeroU64::new(3).unwrap(),
            BudgetEffectState::Settled,
            vec![
                BudgetObservation::new(
                    BudgetDimension::ChildSlots,
                    None,
                    None,
                    BudgetObservationValue::Measured(1),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let settled = event(
            3,
            &job_id,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement.clone()),
            },
        );
        store.settle_coding_budget(settled.clone()).await.unwrap();
        Self {
            root,
            database,
            store,
            job_id,
            repository_id,
            snapshot_id,
            child,
            reservation,
            settlement,
            events: vec![running, reserved, settled],
        }
    }
}

pub(super) async fn assert_coding_seed_survives(
    store: &SqliteStateStore,
    seed: &CodingSeed,
    events: &[CodingEventV1],
) {
    assert_eq!(
        store
            .get_coding_job(&seed.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Running
    );
    assert_eq!(
        store
            .list_coding_children(&seed.job_id, None, 1)
            .await
            .unwrap()
            .items,
        vec![seed.child.clone()]
    );
    assert_eq!(
        store
            .list_coding_events(&seed.job_id, None, 10)
            .await
            .unwrap()
            .events,
        events
    );
    let budget = store
        .get_coding_budget_reservation(&seed.job_id, NonZeroU64::new(2).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.reservation, seed.reservation);
    assert_eq!(budget.settlement, Some(seed.settlement.clone()));
}

fn event(
    sequence: u64,
    job_id: &JobId,
    actor: Option<(AgentId, Option<InvocationId>)>,
    kind: CodingEventKind,
) -> CodingEventV1 {
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        job_id.clone(),
        actor.as_ref().map(|(agent, _)| agent.clone()),
        actor.and_then(|(_, invocation)| invocation),
        kind,
    )
    .unwrap()
}
