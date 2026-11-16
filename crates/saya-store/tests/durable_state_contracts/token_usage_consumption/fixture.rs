use saya_store::{
    CodingApprovalConsumptionContext, NewCodingChild, NewCodingJob, SqliteStateStore,
};
use saya_types::{
    AgentId, ApprovalDisposition, ApprovalNetworkDestination, ApprovalNetworkScheme,
    ApprovalRequestContext, ApprovalRequestV1, ApprovalScope, BudgetCharge, BudgetDimension,
    BudgetReservationV1, CodingEventKind, CodingEventV1, InvocationId, JobId, JobState,
    RepositoryId, Sha256Digest, SnapshotId,
};
use std::{
    fs,
    num::NonZeroU64,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}

pub(super) fn root(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "saya-r13b-{label}-{}-{}",
        std::process::id(),
        now()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

pub(super) fn scope(host: &str) -> ApprovalScope {
    ApprovalScope::network_access(
        ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, host).unwrap(),
    )
}

#[derive(Clone, Copy)]
pub(super) struct RequestSpec<'a> {
    pub digest: &'a str,
    pub epoch: u64,
    pub requested: u64,
    pub expires: u64,
    pub host: &'a str,
}

pub(super) fn event(
    sequence: u64,
    job: &JobId,
    actor: Option<(&AgentId, &InvocationId)>,
    kind: CodingEventKind,
) -> CodingEventV1 {
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        job.clone(),
        actor.map(|(agent, _)| agent.clone()),
        actor.map(|(_, invocation)| invocation.clone()),
        kind,
    )
    .unwrap()
}

pub(super) fn request(
    job: &JobId,
    sequence: u64,
    actor: Option<(&AgentId, &InvocationId)>,
    spec: RequestSpec<'_>,
) -> ApprovalRequestV1 {
    let context = ApprovalRequestContext::new(
        Sha256Digest::parse(&spec.digest.repeat(64)).unwrap(),
        spec.epoch,
        spec.requested,
        spec.expires,
    )
    .unwrap();
    let actual_scope = scope(spec.host);
    let sequence = NonZeroU64::new(sequence).unwrap();
    match actor {
        Some((agent, invocation)) => ApprovalRequestV1::new(
            job.clone(),
            agent.clone(),
            invocation.clone(),
            sequence,
            context,
            actual_scope,
        )
        .unwrap(),
        None => {
            ApprovalRequestV1::new_for_coordinator(job.clone(), sequence, context, actual_scope)
                .unwrap()
        }
    }
}

pub(super) fn request_event(request: &ApprovalRequestV1) -> CodingEventV1 {
    let actor = request.agent_id().zip(request.invocation_id());
    event(
        request.request_sequence().get(),
        request.job_id(),
        actor,
        CodingEventKind::ApprovalRequested {
            request: Box::new(request.clone()),
        },
    )
}

pub(super) fn reservation(
    job: &JobId,
    sequence: u64,
    actor: Option<(&AgentId, &InvocationId)>,
    amount: u64,
) -> CodingEventV1 {
    let reservation = BudgetReservationV1::new(
        job.clone(),
        NonZeroU64::new(sequence).unwrap(),
        actor.map(|(agent, _)| agent.clone()),
        actor.map(|(_, invocation)| invocation.clone()),
        vec![
            BudgetCharge::new(
                BudgetDimension::ToolCalls,
                None,
                None,
                NonZeroU64::new(amount).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    event(
        sequence,
        job,
        actor,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    )
}

pub(super) fn closure(
    job: &JobId,
    request_sequence: u64,
    reservation_sequence: u64,
    actor: Option<(&AgentId, &InvocationId)>,
) -> CodingEventV1 {
    event(
        reservation_sequence + 1,
        job,
        actor,
        CodingEventKind::ApprovalClosed {
            request_sequence: NonZeroU64::new(request_sequence).unwrap(),
            disposition: ApprovalDisposition::Consumed {
                reservation_sequence: NonZeroU64::new(reservation_sequence).unwrap(),
            },
        },
    )
}

pub(super) fn context(request: &ApprovalRequestV1) -> CodingApprovalConsumptionContext {
    CodingApprovalConsumptionContext::new(
        request.policy_sha256().clone(),
        request.revocation_epoch(),
        request.scope().clone(),
    )
}

pub(super) async fn running(label: &str) -> (PathBuf, SqliteStateStore, JobId) {
    let path = root(label);
    let store = SqliteStateStore::new(path.join("state.sqlite3"));
    let job = JobId::parse(&format!("job-r13b-{label}")).unwrap();
    store
        .create_coding_job(NewCodingJob {
            job_id: job.clone(),
            repository_id: RepositoryId::parse(&format!("repository-r13b-{label}")).unwrap(),
            snapshot_id: SnapshotId::parse(&format!("snapshot-r13b-{label}")).unwrap(),
        })
        .await
        .unwrap();
    store
        .append_coding_event(event(
            1,
            &job,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();
    (path, store, job)
}

pub(super) async fn child(
    store: &SqliteStateStore,
    job: &JobId,
    tag: &str,
) -> (AgentId, InvocationId) {
    let agent = AgentId::parse(&format!("agent-r13b-{tag}")).unwrap();
    let invocation = InvocationId::parse(&format!("invocation-r13b-{tag}")).unwrap();
    store
        .create_coding_child(NewCodingChild {
            job_id: job.clone(),
            agent_id: agent.clone(),
            current_invocation_id: invocation.clone(),
        })
        .await
        .unwrap();
    (agent, invocation)
}

pub(super) async fn append_request(
    store: &SqliteStateStore,
    job: &JobId,
    sequence: u64,
    actor: Option<(&AgentId, &InvocationId)>,
    requested: u64,
    expires: u64,
) -> ApprovalRequestV1 {
    let request = request(
        job,
        sequence,
        actor,
        RequestSpec {
            digest: "a",
            epoch: 7,
            requested,
            expires,
            host: "example.com",
        },
    );
    store
        .record_coding_approval_request(request_event(&request))
        .await
        .unwrap();
    request
}
