//! A call that failed earlier in this run is executed again: the run's failure
//! memory counts the repeats so it can offer advice on the third, but it never
//! blocks a call. These drive `run_agent` against a fixture filesystem,
//! asserting the executor sees every attempt and that the retry's own result —
//! never a remembered refusal — is what the model receives.
mod workspace_retry_support;
use saya_agent::{AllowReadOnlyApproval, run_agent};
use workspace_retry_support::*;
#[path = "workspace_retry_support/settlement_tests.rs"]
mod settlement_tests;
use serde_json::json;

/// A read of a missing path is executed again after a mutation, whatever kind
/// of failure it produced and whatever the mutation did. The retry reports the
/// file's real state; a read that still finds nothing fails with the
/// executor's own error rather than a remembered refusal.
#[tokio::test]
async fn a_read_of_a_missing_path_is_retried_whatever_the_failure() {
    for (failure, write_path, created) in [
        ("typed", "a.txt", true),
        ("wrapped", "a.txt", true),
        ("typed", "other.txt", false),
        ("opaque", "a.txt", true),
        ("opaque_wrapped", "a.txt", true),
        ("deterministic", "a.txt", true),
        ("write_refused", "a.txt", false),
        ("write_unknown", "a.txt", true),
    ] {
        for mutation in ["write", "edit"] {
            let files = Files::new(failure);
            let provider = Script::new(vec![
                turn(vec![call("first", "read", "a.txt")]),
                turn(vec![call("mutation", mutation, write_path)]),
                turn(vec![call("retry", "read", "a.txt")]),
                done(),
            ]);
            run_agent(
                &provider,
                &files,
                request(),
                definitions(false),
                limits(),
                &AllowReadOnlyApproval,
            )
            .await
            .unwrap();
            assert!(provider.result("first")["error"].is_string());
            assert_eq!(
                files.read_count("a.txt"),
                2,
                "{failure}/{mutation}: every retry executes"
            );
            let retry = provider.result("retry");
            if created {
                assert_eq!(retry["content"], "created\n", "{failure}/{mutation}");
            } else {
                assert!(retry["error"].is_string(), "{failure}/{mutation}: {retry}");
                assert!(
                    !retry["error"].as_str().unwrap().contains("already failed"),
                    "{failure}/{mutation}: a repeat is never refused: {retry}"
                );
            }
        }
    }
}

/// Every repeat of a failed read is executed, however many there are: a second
/// retry after a failed retry runs too, and neither carries a remembered
/// refusal.
#[tokio::test]
async fn every_repeat_of_a_failed_read_is_executed() {
    let files = Files::new("typed");
    let mut provider = Script::new(vec![
        turn(vec![call("first", "read", "a.txt")]),
        turn(vec![call("create", "write", "a.txt")]),
        turn(vec![call("retry", "read", "a.txt")]),
        turn(vec![call("again", "read", "a.txt")]),
        done(),
    ]);
    provider.remove_before_retry = Some(files.root.join("a.txt"));
    run_agent(
        &provider,
        &files,
        request(),
        definitions(false),
        limits(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(files.read_count("a.txt"), 3);
    for id in ["retry", "again"] {
        let error = provider.result(id)["error"]
            .as_str()
            .unwrap_or_else(|| panic!("{id} fails with the executor's own error"))
            .to_owned();
        assert!(!error.contains("already failed"), "{id}: {error}");
    }
}

/// Two reads of the same path with different arguments are two different
/// calls, and each is retried on its own: the mutation decides nothing for
/// either of them.
#[tokio::test]
async fn distinct_argument_keys_are_each_retried() {
    let files = Files::new("typed");
    let mut first = call("first", "read", "a.txt");
    first.arguments["view"] = json!(1);
    let mut second = call("second", "read", "a.txt");
    second.arguments["view"] = json!(2);
    let mut retry_first = first.clone();
    retry_first.id = "retry-first".into();
    let mut retry_second = second.clone();
    retry_second.id = "retry-second".into();
    let provider = Script::new(vec![
        turn(vec![first, second]),
        turn(vec![call("create", "write", "a.txt")]),
        turn(vec![retry_second, retry_first]),
        done(),
    ]);
    run_agent(
        &provider,
        &files,
        request(),
        definitions(false),
        limits(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(files.read_count("a.txt"), 4);
    assert_eq!(provider.result("retry-second")["content"], "created\n");
    assert_eq!(provider.result("retry-first")["content"], "created\n");
}

/// Concurrent absent reads are each retried after the serial mutation: the one
/// whose path was created succeeds, the other reports its own missing-file
/// error.
#[tokio::test]
async fn concurrent_absent_reads_are_retried() {
    let mut files = Files::new("typed");
    files.barrier = Some(tokio::sync::Barrier::new(2));
    let provider = Script::new(vec![
        turn(vec![call("a", "read", "a.txt"), call("b", "read", "b.txt")]),
        turn(vec![call("create", "write", "a.txt")]),
        turn(vec![
            call("retry-a", "read", "a.txt"),
            call("retry-b", "read", "b.txt"),
        ]),
        done(),
    ]);
    run_agent(
        &provider,
        &files,
        request(),
        definitions(true),
        limits(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(files.read_count("a.txt"), 2);
    assert_eq!(files.read_count("b.txt"), 2);
    assert_eq!(provider.result("retry-a")["content"], "created\n");
    assert!(provider.result("retry-b")["error"].is_string());
}

/// A read that overlapped the write which created its file is retried too: the
/// retry sees the file the concurrent write left behind.
#[tokio::test]
async fn a_read_overlapping_a_write_is_still_retried() {
    let mut files = Files::new("overlap_mutation");
    files.barrier = Some(tokio::sync::Barrier::new(2));
    let provider = Script::new(vec![
        turn(vec![
            call("missing", "read", "a.txt"),
            call("create", "write", "a.txt"),
        ]),
        turn(vec![call("retry", "read", "a.txt")]),
        done(),
    ]);
    let mut definitions = definitions(true);
    definitions
        .iter_mut()
        .find(|definition| definition.name == "write")
        .unwrap()
        .concurrency = saya_agent::ToolConcurrency::Concurrent;
    run_agent(
        &provider,
        &files,
        request(),
        definitions,
        limits(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(files.read_count("a.txt"), 2);
    assert_eq!(files.writes.load(std::sync::atomic::Ordering::Acquire), 1);
    assert_eq!(
        std::fs::read(files.root.join("a.txt")).unwrap(),
        b"created\n"
    );
    assert!(provider.result("missing")["error"].is_string());
    assert!(provider.result("create")["digest"].is_string());
    assert_eq!(provider.result("retry")["content"], "created\n");
}

/// A long path is retried like any other: the bound on how a remembered key is
/// rendered decides nothing about whether a call runs.
#[tokio::test]
async fn long_paths_are_retried_like_any_other() {
    for extra in [0, 1] {
        let files = Files::new("typed");
        let path = format!(
            "{}/{}/{}",
            "a".repeat(170),
            "b".repeat(170),
            "c".repeat(170 + extra)
        );
        assert_eq!(path.len(), 512 + extra);
        std::fs::create_dir_all(files.root.join(&path).parent().unwrap()).unwrap();
        let provider = Script::new(vec![
            turn(vec![call("first", "read", &path)]),
            turn(vec![call("mutation", "write", &path)]),
            turn(vec![call("retry", "read", &path)]),
            done(),
        ]);
        run_agent(
            &provider,
            &files,
            request(),
            definitions(false),
            limits(),
            &AllowReadOnlyApproval,
        )
        .await
        .unwrap();
        assert_eq!(files.read_count(&path), 2, "extra={extra}");
        assert_eq!(provider.result("retry")["content"], "created\n");
    }
}
