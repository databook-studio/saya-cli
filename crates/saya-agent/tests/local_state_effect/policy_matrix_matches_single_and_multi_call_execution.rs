/// Every execution route consumes the same effect, permit, and approval
/// policy: denied calls never reach the executor, while allowed calls run
/// once each in both one-call and multi-call turns. Each case runs with serial
/// and explicitly concurrent metadata, so the P1 policy parity also covers
/// the scheduler's batch-eligibility decision.
#[tokio::test]
async fn policy_matrix_matches_single_and_multi_call_execution() {
    for local_state in [
        LocalStateEffect::None,
        LocalStateEffect::Read,
        LocalStateEffect::WriteCandidate,
        LocalStateEffect::WriteWorkspace,
        LocalStateEffect::WriteSession,
    ] {
        for database_data in [false, true] {
            for external_side_effect in [false, true] {
                for requires_approval in [false, true] {
                    for permits in 0..8 {
                        let limits = AgentLimits {
                            permit_candidate_writes: permits & 1 != 0,
                            permit_workspace_writes: permits & 2 != 0,
                            permit_external_effects: permits & 4 != 0,
                            ..AgentLimits::default()
                        };
                        for approved in [false, true] {
                            let effect = ToolEffect {
                                database_data,
                                external_side_effect,
                                requires_approval,
                                local_state,
                            };
                            let (status, denial) = expected_policy(effect, limits, approved);
                            let approval: &dyn ApprovalDecider = if approved {
                                &AllowApproval
                            } else {
                                &DenyApproval
                            };
                            for concurrency in
                                [ToolConcurrency::Serial, ToolConcurrency::Concurrent]
                            {
                                for call_count in [1, 2] {
                                    let (executed, reasons, statuses) = run_policy_turn(
                                        call_count,
                                        effect,
                                        concurrency,
                                        limits,
                                        approval,
                                    )
                                    .await;
                                    assert_eq!(
                                        executed,
                                        if denial.is_some() { 0 } else { call_count },
                                        "{effect:?}, {concurrency:?}, permits={permits}, approved={approved}, {call_count} calls"
                                    );
                                    assert_eq!(
                                        statuses,
                                        vec![status; call_count],
                                        "{effect:?}, {concurrency:?}, permits={permits}, approved={approved}, {call_count} calls"
                                    );
                                    match denial {
                                        Some(reason) => assert_eq!(
                                            reasons,
                                            vec![reason; call_count],
                                            "{effect:?}, {concurrency:?}, permits={permits}, approved={approved}, {call_count} calls"
                                        ),
                                        None => assert!(
                                            reasons.is_empty(),
                                            "{effect:?}, {concurrency:?}, permits={permits}, approved={approved}, {call_count} calls: {reasons:?}"
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
