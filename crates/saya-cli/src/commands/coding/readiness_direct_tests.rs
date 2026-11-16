//! The pure shapers behind the direct advisory readiness: the change kinds and
//! the bounded completion-history walk. Both are bounded, and neither can carry
//! program text.

use super::{MAX_COMPLETION_HISTORY_PAGES, change_kind, latest_failure_classes};
use saya_harness::coding::integration::DeltaKind;
use saya_store::{CodingCompletion, CodingCompletionPage, StoreError};
use saya_types::{
    AgentId, CodingFailureClass, CodingResultOutcome, InvocationId, JobId, JoinedResultV1,
};
use std::{
    collections::BTreeSet,
    num::NonZeroU64,
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn every_change_kind_is_named_for_the_model() {
    assert_eq!(change_kind(DeltaKind::Added), "created");
    assert_eq!(change_kind(DeltaKind::Modified), "modified");
    assert_eq!(change_kind(DeltaKind::Deleted), "deleted");
}

fn agent(value: &str) -> AgentId {
    AgentId::parse(value).unwrap()
}

fn wanted(names: &[&str]) -> BTreeSet<AgentId> {
    names.iter().map(|name| agent(name)).collect()
}

fn completion(
    agent_name: &str,
    sequence: u64,
    outcome: CodingResultOutcome,
    class: Option<CodingFailureClass>,
) -> CodingCompletion {
    let result = JoinedResultV1::new(
        JobId::parse("job-1").unwrap(),
        agent(agent_name),
        InvocationId::parse("inv-1").unwrap(),
        outcome,
        String::new(),
        false,
    )
    .unwrap();
    CodingCompletion {
        sequence: NonZeroU64::new(sequence).unwrap(),
        result: match class {
            Some(class) => result.with_failure_class(class),
            None => result,
        },
        acknowledged: false,
    }
}

fn page(items: Vec<CodingCompletion>, next: Option<u64>) -> CodingCompletionPage {
    CodingCompletionPage {
        items,
        next_cursor: next.map(|cursor| NonZeroU64::new(cursor).unwrap()),
    }
}

#[tokio::test]
async fn the_completion_history_walk_stops_at_its_page_cap() {
    // One page more than the cap, each carrying a completion for the wanted
    // child: a walk that ignored the cap would read every one of them.
    let pages = (0..=MAX_COMPLETION_HISTORY_PAGES)
        .map(|index| {
            page(
                vec![completion(
                    "child-a",
                    index as u64 + 1,
                    CodingResultOutcome::Completed,
                    None,
                )],
                Some(index as u64 + 1),
            )
        })
        .collect::<Vec<_>>();
    let served = AtomicUsize::new(0);
    let walked = latest_failure_classes(&wanted(&["child-a"]), |_| {
        let index = served.fetch_add(1, Ordering::AcqRel);
        let next = pages.get(index).cloned();
        async move { next.ok_or(StoreError::NotFound) }
    })
    .await;
    assert_eq!(
        served.load(Ordering::Acquire),
        MAX_COMPLETION_HISTORY_PAGES,
        "the walk stops at the cap instead of reading the whole history"
    );
    assert_eq!(walked.classes.len(), 1, "{:?}", walked.classes);
    assert!(
        walked.history_truncated,
        "the walk hit its cap, so a later completion may carry a later class"
    );
}

#[tokio::test]
async fn a_history_the_walk_reads_to_the_end_is_not_flagged_truncated() {
    let single = page(
        vec![completion(
            "child-a",
            1,
            CodingResultOutcome::Failed,
            Some(CodingFailureClass::ProviderError),
        )],
        None,
    );
    let walked = latest_failure_classes(&wanted(&["child-a"]), |_| {
        let next = single.clone();
        async move { Ok(next) }
    })
    .await;
    assert!(
        !walked.history_truncated,
        "a walk that ended at the last page read the whole history"
    );
    assert_eq!(
        walked.classes[&agent("child-a")],
        Some(CodingFailureClass::ProviderError)
    );
}

#[tokio::test]
async fn a_page_that_cannot_be_read_leaves_the_classes_incomplete() {
    // A store read that fails ends the walk with what it has: the classes are
    // unknown-complete, so the walk reports itself truncated rather than
    // claiming the latest completion won.
    let walked = latest_failure_classes(&wanted(&["child-a"]), |_| async move {
        Err(StoreError::Unavailable)
    })
    .await;
    assert!(walked.classes.is_empty(), "{:?}", walked.classes);
    assert!(
        walked.history_truncated,
        "an unread page may hold a later class"
    );
}

#[tokio::test]
async fn only_the_wanted_children_are_kept() {
    let child_a = page(
        vec![
            completion(
                "child-a",
                1,
                CodingResultOutcome::Failed,
                Some(CodingFailureClass::ProviderStreamEnded),
            ),
            completion(
                "child-b",
                2,
                CodingResultOutcome::Failed,
                Some(CodingFailureClass::ProviderStreamEnded),
            ),
        ],
        None,
    );
    let walked = latest_failure_classes(&wanted(&["child-a"]), |_| {
        let next = child_a.clone();
        async move { Ok(next) }
    })
    .await;
    let classes = walked.classes;
    assert_eq!(
        classes.len(),
        1,
        "an agent outside the children listing the caller read is dropped: {classes:?}"
    );
    assert_eq!(
        classes[&agent("child-a")],
        Some(CodingFailureClass::ProviderStreamEnded)
    );
}

#[tokio::test]
async fn a_child_that_failed_then_completed_reports_no_class() {
    let failed = page(
        vec![completion(
            "child-a",
            1,
            CodingResultOutcome::Failed,
            Some(CodingFailureClass::ProviderStreamEnded),
        )],
        Some(1),
    );
    let completed = page(
        vec![completion(
            "child-a",
            2,
            CodingResultOutcome::Completed,
            None,
        )],
        None,
    );
    let walked = latest_failure_classes(&wanted(&["child-a"]), move |cursor| {
        let next = if cursor.is_none() {
            failed.clone()
        } else {
            completed.clone()
        };
        async move { Ok(next) }
    })
    .await;
    let classes = walked.classes;
    assert_eq!(
        classes[&agent("child-a")],
        None,
        "the latest completion wins, so the class of the earlier failure is gone: {classes:?}"
    );
}
