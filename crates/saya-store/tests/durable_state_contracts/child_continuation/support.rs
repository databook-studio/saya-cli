use super::super::{event_projection_storage::event, job_metadata_storage::child};
use saya_store::{CodingChildRecord, CodingRuntimeAuthority};
use saya_types::{
    ChildInvocationRecord, CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId,
    JobState, JoinedResultV1, ManagedCheckoutId, Sha256Digest,
};
use std::num::NonZeroU64;
#[path = "checkout.rs"]
mod checkout;
pub struct Fixture {
    pub base: checkout::Fixture,
    pub child: CodingChildRecord,
    pub authority: CodingRuntimeAuthority,
    pub next: u64,
}
pub fn nz(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}
pub fn digest(value: &str) -> Sha256Digest {
    Sha256Digest::parse(&value.repeat(64)).unwrap()
}
impl Fixture {
    pub async fn new(label: &str) -> Self {
        Self::with_binding(label, true).await
    }
    pub async fn legacy(label: &str) -> Self {
        Self::with_binding(label, false).await
    }
    async fn with_binding(label: &str, bound: bool) -> Self {
        let base = checkout::Fixture::new(label).await;
        let child = base
            .store
            .create_coding_child(child(base.job.clone(), label))
            .await
            .unwrap();
        let create = base.create_event(
            2,
            "checkout-child",
            base.repository.clone(),
            base.snapshot.clone(),
            Some(child.agent_id.clone()),
        );
        base.store
            .record_checkout_create_intent(create)
            .await
            .unwrap();
        let done = base.operation_event(
            3,
            2,
            "checkout-child",
            saya_types::RepositoryMutationAction::CreateCheckout,
            saya_types::CheckoutOperationState::Completed,
            Some(child.agent_id.clone()),
        );
        base.store
            .record_checkout_operation_outcome(done)
            .await
            .unwrap();
        let authority = CodingRuntimeAuthority {
            epoch: 1,
            policy_digest: digest("a"),
        };
        base.store
            .bind_coding_runtime(&base.job, &authority)
            .await
            .unwrap();
        if bound {
            let scope = event(
                4,
                &base.job,
                Some(&child),
                CodingEventKind::ChildInvocation {
                    record: Box::new(ChildInvocationRecord::ScopeBound {
                        checkout_id: ManagedCheckoutId::parse("checkout-child").unwrap(),
                        scope_sha256: digest("b"),
                    }),
                },
            );
            base.store
                .record_coding_child_invocation(scope, &authority)
                .await
                .unwrap();
        }
        base.store
            .append_coding_event(event(
                if bound { 5 } else { 4 },
                &base.job,
                Some(&child),
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            ))
            .await
            .unwrap();
        Self {
            base,
            child,
            authority,
            next: if bound { 6 } else { 5 },
        }
    }
    pub fn event(&mut self, kind: CodingEventKind) -> CodingEventV1 {
        let result = event(self.next, &self.base.job, Some(&self.child), kind);
        self.next += 1;
        result
    }
    pub async fn complete(&mut self) -> NonZeroU64 {
        let result = JoinedResultV1::new(
            self.base.job.clone(),
            self.child.agent_id.clone(),
            self.child.current_invocation_id.clone(),
            CodingResultOutcome::Completed,
            "finished".into(),
            false,
        )
        .unwrap();
        let event = self.event(CodingEventKind::StateChanged {
            state: JobState::Completed,
        });
        let sequence = event.sequence();
        self.base
            .store
            .record_coding_completion(event, &result)
            .await
            .unwrap();
        sequence
    }
    pub fn prepared(&mut self, completion: NonZeroU64) -> CodingEventV1 {
        self.prepared_index(completion, 1)
    }
    pub fn prepared_index(&mut self, completion: NonZeroU64, index: u8) -> CodingEventV1 {
        CodingEventV1::new(
            nz(self.next),
            self.base.job.clone(),
            Some(self.child.agent_id.clone()),
            Some(InvocationId::parse(&format!("inv-follow-up-{index}-{}", self.next)).unwrap()),
            CodingEventKind::ChildInvocation {
                record: Box::new(ChildInvocationRecord::FollowUpPrepared {
                    checkout_id: ManagedCheckoutId::parse("checkout-child").unwrap(),
                    previous_invocation_id: self.child.current_invocation_id.clone(),
                    previous_completion_sequence: completion,
                    source_sha256: digest("c"),
                    scope_sha256: digest("b"),
                    instruction_sha256: digest("d"),
                    follow_up_index: index,
                }),
            },
        )
        .unwrap()
    }
    pub async fn rotate_and_start(&mut self, event: CodingEventV1) {
        self.base
            .store
            .record_coding_child_invocation(event.clone(), &self.authority)
            .await
            .unwrap();
        self.child.current_invocation_id = event.invocation_id().unwrap().clone();
        self.next += 1;
        let running = self.event(CodingEventKind::StateChanged {
            state: JobState::Running,
        });
        self.base.store.append_coding_event(running).await.unwrap();
    }
    pub async fn assert_refused(&self, event: CodingEventV1) {
        assert!(matches!(
            self.base
                .store
                .record_coding_child_invocation(event, &self.authority)
                .await,
            Err(saya_store::ChildInvocationError::Refused)
        ));
        let child = self
            .base
            .store
            .list_coding_children(&self.base.job, None, 100)
            .await
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(
            child.current_invocation_id,
            self.child.current_invocation_id
        );
        assert_eq!(child.state, JobState::Completed);
    }
}
