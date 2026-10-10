use saya_types::{
    AgentId, InvocationId, JobId, JobState, JobStateTransitionError, ManagedCheckoutId,
    RepositoryId, SnapshotId,
};
use serde::de::DeserializeOwned;

#[test]
fn job_wire_rejects_invalid_transition() {
    let states = [
        JobState::Queued,
        JobState::Running,
        JobState::WaitingForApproval,
        JobState::Paused,
        JobState::Completed,
        JobState::Failed,
        JobState::Cancelled,
        JobState::Uncertain,
    ];
    let allowed = [
        (JobState::Queued, JobState::Running),
        (JobState::Queued, JobState::Failed),
        (JobState::Queued, JobState::Cancelled),
        (JobState::Running, JobState::WaitingForApproval),
        (JobState::Running, JobState::Paused),
        (JobState::Running, JobState::Completed),
        (JobState::Running, JobState::Failed),
        (JobState::Running, JobState::Cancelled),
        (JobState::Running, JobState::Uncertain),
        (JobState::WaitingForApproval, JobState::Queued),
        (JobState::WaitingForApproval, JobState::Failed),
        (JobState::WaitingForApproval, JobState::Cancelled),
        (JobState::WaitingForApproval, JobState::Uncertain),
    ];

    for from in states {
        for to in states {
            let expected = if allowed.contains(&(from, to)) {
                Ok(to)
            } else {
                Err(JobStateTransitionError::InvalidTransition { from, to })
            };
            assert_eq!(from.transition_to(to), expected, "{from:?} -> {to:?}");
        }
    }
}

fn rejects_invalid_wire_values<T: DeserializeOwned>() {
    let oversized = format!("\"{}\"", "a".repeat(129));
    for wire in [
        "\"\"",
        "\"../outside\"",
        "\"has space\"",
        "\"bad\\tid\"",
        "\"caf\\u00e9\"",
        "\"has.dot\"",
        oversized.as_str(),
    ] {
        assert!(serde_json::from_str::<T>(wire).is_err(), "accepted {wire}");
    }
}

#[test]
fn every_coding_id_rejects_invalid_wire_values() {
    rejects_invalid_wire_values::<JobId>();
    rejects_invalid_wire_values::<AgentId>();
    rejects_invalid_wire_values::<InvocationId>();
    rejects_invalid_wire_values::<RepositoryId>();
    rejects_invalid_wire_values::<SnapshotId>();
    rejects_invalid_wire_values::<ManagedCheckoutId>();
}

#[test]
fn coding_ids_and_states_have_stable_wire_shapes() {
    assert_eq!(
        serde_json::to_string(&JobId::parse("job_1").unwrap()).unwrap(),
        "\"job_1\""
    );
    assert_eq!(
        serde_json::to_string(&AgentId::parse("agent-1").unwrap()).unwrap(),
        "\"agent-1\""
    );
    assert_eq!(
        serde_json::to_string(&InvocationId::parse("invocation-1").unwrap()).unwrap(),
        "\"invocation-1\""
    );
    assert_eq!(
        serde_json::to_string(&RepositoryId::parse("repo-1").unwrap()).unwrap(),
        "\"repo-1\""
    );
    assert_eq!(
        serde_json::to_string(&SnapshotId::parse("snapshot-1").unwrap()).unwrap(),
        "\"snapshot-1\""
    );
    assert_eq!(
        serde_json::to_string(&ManagedCheckoutId::parse("checkout-1").unwrap()).unwrap(),
        "\"checkout-1\""
    );

    for (state, wire) in [
        (JobState::Queued, "queued"),
        (JobState::Running, "running"),
        (JobState::WaitingForApproval, "waiting_for_approval"),
        (JobState::Paused, "paused"),
        (JobState::Completed, "completed"),
        (JobState::Failed, "failed"),
        (JobState::Cancelled, "cancelled"),
        (JobState::Uncertain, "uncertain"),
    ] {
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            format!("\"{wire}\"")
        );
        assert_eq!(
            serde_json::from_str::<JobState>(&format!("\"{wire}\"")).unwrap(),
            state
        );
    }
    assert!(serde_json::from_str::<JobState>("\"unknown\"").is_err());
}
