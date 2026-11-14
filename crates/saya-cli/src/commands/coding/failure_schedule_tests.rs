use super::rank::class_rank;
use super::*;
use saya_agent::{AgentError, ProviderError};
use saya_harness::coding::child::OwnerFailureSnapshot;
use saya_harness::coding::coordinator::{
    ChildCleanupFailure, ChildRunError, ScheduleChildError, ScheduleOutcome,
};

#[test]
fn a_nested_owner_operation_latches_its_own_primary_and_secondary() {
    let mut failures = FailureState::default();
    let outcome = ScheduleOutcome {
        agent_id: saya_types::AgentId::parse("child-a").unwrap(),
        invocation_id: saya_types::InvocationId::parse("invocation-a").unwrap(),
        result: Err(ScheduleChildError::Run(ChildRunError::Cleanup {
            primary: Box::new(ChildRunError::OwnerOperation {
                prior: Some(Box::new(ChildRunError::Agent(AgentError::Provider(
                    ProviderError::InvalidResponse,
                )))),
                failure: OwnerFailureSnapshot::new(
                    Class::ExtensionExchange,
                    Some(Class::MandatoryPersistence),
                ),
            }),
            kind: ChildCleanupFailure::BudgetPersistence,
        })),
        completion_sequence: None,
        completion_error: None,
    };

    failures.latch_schedule_outcome(&outcome);

    // The child's own class (the provider error behind the owner operation) is
    // reported in its summary, never latched as the job's failure. Both classes
    // the owner snapshot carries ARE latched; the rank decides their roles, so
    // the persistence failure is the primary and the exchange failure the
    // secondary whatever order the two arrive in.
    assert_eq!(
        failures.primary.as_ref().map(|failure| failure.class),
        Some(Class::MandatoryPersistence)
    );
    assert_eq!(
        failures
            .primary
            .as_ref()
            .and_then(|failure| failure.agent_id.clone()),
        Some(saya_types::AgentId::parse("child-a").unwrap()),
        "the latched failure keeps the child's attribution"
    );
    assert_eq!(failures.secondary, Some(Class::ExtensionExchange));
}

#[test]
fn a_cleanup_failure_latches_when_the_owner_snapshot_repeats_the_child_class() {
    let mut failures = FailureState::default();
    let outcome = ScheduleOutcome {
        agent_id: saya_types::AgentId::parse("child-a").unwrap(),
        invocation_id: saya_types::InvocationId::parse("invocation-a").unwrap(),
        result: Err(ScheduleChildError::Run(ChildRunError::Cleanup {
            primary: Box::new(ChildRunError::OwnerOperation {
                prior: None,
                failure: OwnerFailureSnapshot::new(Class::ExtensionExchange, None),
            }),
            kind: ChildCleanupFailure::Progress,
        })),
        completion_sequence: None,
        completion_error: None,
    };

    failures.latch_schedule_outcome(&outcome);

    // The owner snapshot repeats the child's class, so it is not latched; the
    // child's cleanup failure is the job's failure.
    assert_eq!(
        failures.primary.as_ref().map(|failure| failure.class),
        Some(Class::ProgressDelivery)
    );
    assert_eq!(failures.secondary, None);
}

/// The primary and secondary one arrival order of `order` latches.
fn latched(order: &[Class]) -> (Option<Class>, Option<Class>) {
    let mut failures = FailureState::default();
    for class in order {
        failures.latch(root(*class));
    }
    (
        failures.primary.as_ref().map(|failure| failure.class),
        failures.secondary,
    )
}

/// Every ordering of `set`, as index permutations (the set is small).
fn orderings(set: &[Class]) -> Vec<Vec<Class>> {
    let mut orders = Vec::new();
    let mut indices = vec![0usize; set.len()];
    loop {
        if indices
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == set.len()
        {
            orders.push(indices.iter().map(|index| set[*index]).collect());
        }
        let mut position = 0;
        loop {
            if position == set.len() {
                return orders;
            }
            indices[position] += 1;
            if indices[position] < set.len() {
                break;
            }
            indices[position] = 0;
            position += 1;
        }
    }
}

#[test]
fn every_arrival_order_of_one_failure_set_yields_the_same_primary_and_secondary() {
    // The classes a direct child failure was observed to latch in either order
    // (ledger row 37): the same SET must always yield the same pair.
    let set = [
        Class::EffectsUncertain,
        Class::AdmissionDenied,
        Class::Internal,
    ];
    let orders = orderings(&set);
    assert_eq!(orders.len(), 6, "every permutation is covered");
    let first = latched(&orders[0]);
    for order in &orders {
        assert_eq!(
            latched(order),
            first,
            "the same failures latched in the order {order:?} must yield the same \
             primary and secondary as {first:?}"
        );
    }
    assert_eq!(
        first,
        (Some(Class::EffectsUncertain), Some(Class::AdmissionDenied)),
        "the integrity class outranks the refusal, which outranks the generic class"
    );

    // A second set, four classes wide, over different severities: the rule is
    // the rank of each class, not the size of the set.
    let set = [
        Class::Internal,
        Class::CoordinatorStalled,
        Class::ProviderError,
        Class::MandatoryPersistence,
    ];
    let orders = orderings(&set);
    assert_eq!(orders.len(), 24, "every permutation is covered");
    let first = latched(&orders[0]);
    for order in &orders {
        assert_eq!(latched(order), first, "order {order:?} must not matter");
    }
    assert_eq!(
        first,
        (
            Some(Class::MandatoryPersistence),
            Some(Class::ProviderError)
        )
    );
}

/// Every class the rank table covers, so a new class added to the enum without
/// a rank is caught here rather than by an unstable primary in the field.
const EVERY_CLASS: [Class; 19] = [
    Class::OutputLimit,
    Class::ProviderError,
    Class::ProviderStreamEnded,
    Class::LimitReached,
    Class::InvalidToolCall,
    Class::AdmissionDenied,
    Class::Unsupported,
    Class::SetupFailed,
    Class::Cancelled,
    Class::DiagnosticStorage,
    Class::ProgressDelivery,
    Class::MandatoryPersistence,
    Class::ExtensionExchange,
    Class::EffectsUncertain,
    Class::BudgetStopped,
    Class::DependencyBlocked,
    Class::CoordinatorNoProgress,
    Class::CoordinatorStalled,
    Class::Internal,
];

#[test]
fn the_rank_is_a_strict_total_order_over_every_class() {
    let mut ranks = std::collections::BTreeMap::new();
    for class in EVERY_CLASS {
        let rank = class_rank(class);
        assert!(
            ranks.insert(rank, class).is_none(),
            "{class:?} shares its rank {rank} with another class: the order is not total"
        );
        assert_eq!(
            latched(&[class]),
            (Some(class), None),
            "one latched class is the primary, with no secondary"
        );
    }
    assert_eq!(ranks.len(), EVERY_CLASS.len(), "every class is ranked once");
    for first in EVERY_CLASS {
        for second in EVERY_CLASS {
            if first == second {
                continue;
            }
            let (primary, secondary) = latched(&[first, second]);
            let (reversed_primary, reversed_secondary) = latched(&[second, first]);
            assert_eq!(
                (primary, secondary),
                (reversed_primary, reversed_secondary),
                "{first:?} then {second:?} must latch like {second:?} then {first:?}"
            );
            let (higher, lower) = if class_rank(first) > class_rank(second) {
                (first, second)
            } else {
                (second, first)
            };
            assert_eq!(
                (primary, secondary),
                (Some(higher), Some(lower)),
                "the higher-ranked of {first:?} and {second:?} is the primary"
            );
        }
    }
}
