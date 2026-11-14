//! Actual independent processes share the stable lock across renamed segments.
use super::{
    tests::{record, root},
    *,
};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
const EXCLUSIVE_RECORDS: u64 = 160;

fn append_many(path: &Path) -> u64 {
    let mut successes = 0;
    for _ in 0..500 {
        match append(path, Stream::ToolEvents, &record()) {
            Ok(()) => successes += 1,
            Err(Fault::LockContended) => std::thread::yield_now(),
            Err(other) => panic!("unexpected diagnostic failure: {other:?}"),
        }
    }
    successes
}
const LOCK_RETRY: Duration = Duration::from_secs(30);

/// The product's contended lock is a documented retryable fault, never a
/// failure: the handshake sequences the exclusive phases, but the lock is
/// still the only arbitration, so a peer's late release is retried here
/// exactly as `append_many` retries it. The wait is bounded so a stuck peer
/// fails the test instead of hanging it.
fn retry_contended<T>(mut attempt: impl FnMut() -> Result<T, Fault>) -> T {
    let started = std::time::Instant::now();
    loop {
        match attempt() {
            Ok(value) => return value,
            Err(Fault::LockContended) if started.elapsed() < LOCK_RETRY => std::thread::yield_now(),
            Err(other) => panic!("unexpected diagnostic failure: {other:?}"),
        }
    }
}
fn append_exact(path: &Path) {
    for _ in 0..EXCLUSIVE_RECORDS {
        retry_contended(|| append(path, Stream::ToolEvents, &record()));
    }
}
fn signal(root: &Path, name: &str, value: &serde_json::Value) {
    let stage = root.join(format!("{name}.stage"));
    std::fs::write(&stage, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::rename(stage, root.join(name)).unwrap();
}
fn wait_signal(root: &Path, name: &str) -> serde_json::Value {
    let path = root.join(name);
    for _ in 0..3000 {
        match std::fs::read(&path) {
            Ok(bytes) => return serde_json::from_slice(&bytes).unwrap(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("handshake read failed: {error}"),
        }
    }
    panic!("independent process did not signal {name} within 3000 polls");
}
fn wait_child(child: &mut Child) {
    for _ in 0..3000 {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "independent writer failed: {status}");
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("independent writer exceeded 3000 completion polls");
}
fn generation(path: &Path) -> u64 {
    retry_contended(|| history(path).map(|history| history.streams[0].generation.unwrap()))
}
#[test]
fn independent_processes_append_rotate_and_account_for_every_success() {
    const CHILD: &str = "SAYA_TEST_DIAGNOSTIC_PROCESS_ROOT";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = Path::new(&path);
        // Parent waits: child must first write and rotate, never zero admissions.
        append_exact(path);
        let initial_generation = generation(path);
        assert!(initial_generation >= 1);
        signal(path, "child-ready", &serde_json::json!(initial_generation));
        let parent_generation = wait_signal(path, "parent-go").as_u64().unwrap();
        let observed_generation = generation(path);
        assert_eq!(observed_generation, parent_generation);
        assert!(observed_generation > initial_generation);
        signal(
            path,
            "child-observed",
            &serde_json::json!(observed_generation),
        );
        let parallel = append_many(path);
        wait_signal(path, "parent-done");
        let before_tail = generation(path);
        // Parent has finished: child must observe its rotations and rotate again.
        append_exact(path);
        let final_generation = generation(path);
        assert!(final_generation > before_tail);
        signal(
            path,
            "child-result",
            &serde_json::json!({
                "initial":EXCLUSIVE_RECORDS,"parallel":parallel,"tail":EXCLUSIVE_RECORDS,
                "initial_generation":initial_generation,"observed_generation":observed_generation,
                "before_tail_generation":before_tail,"final_generation":final_generation
            }),
        );
        return;
    }
    let root = root();
    // Initialize before contention; neither writer may mistake valid history for legacy.
    append(root.path(), Stream::ToolEvents, &record()).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "commands::coding::diagnostics::process_tests::independent_processes_append_rotate_and_account_for_every_success"])
        .env(CHILD, root.path())
        .stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().unwrap();
    let child_generation = wait_signal(root.path(), "child-ready").as_u64().unwrap();
    // Child waits: parent must write after observing the child's renamed segments.
    assert_eq!(generation(root.path()), child_generation);
    append_exact(root.path());
    let parent_generation = generation(root.path());
    assert!(parent_generation > child_generation);
    signal(
        root.path(),
        "parent-go",
        &serde_json::json!(parent_generation),
    );
    assert_eq!(
        wait_signal(root.path(), "child-observed"),
        parent_generation
    );
    let own_parallel = append_many(root.path());
    signal(root.path(), "parent-done", &serde_json::json!(true));
    wait_child(&mut child);
    let child = wait_signal(root.path(), "child-result");
    let child_total = child["initial"].as_u64().unwrap()
        + child["parallel"].as_u64().unwrap()
        + child["tail"].as_u64().unwrap();
    let own = EXCLUSIVE_RECORDS + own_parallel;
    assert!(own > 0 && child_total > 0);
    assert_eq!(child["initial"], EXCLUSIVE_RECORDS);
    assert_eq!(child["tail"], EXCLUSIVE_RECORDS);
    assert_eq!(child["observed_generation"], parent_generation);
    assert!(
        child["final_generation"].as_u64().unwrap()
            > child["before_tail_generation"].as_u64().unwrap()
    );
    let snapshot = history(root.path()).unwrap();
    assert_ne!(snapshot.state, HistoryState::Unknown);
    let stream = &snapshot.streams[0];
    let retained = stream.current.records
        + stream
            .previous
            .as_ref()
            .map_or(0, |segment| segment.records);
    assert_eq!(
        retained + stream.evicted_records.unwrap(),
        own + child_total + 1
    );
    assert!(
        stream.generation.unwrap() >= 1,
        "must exercise actual cross-process rotation"
    );
    assert_eq!(
        stream.generation.unwrap(),
        child["final_generation"].as_u64().unwrap()
    );
    assert!(stream.current.bytes <= SEGMENT_BYTES as u64);
    assert!(stream.previous.as_ref().unwrap().bytes <= SEGMENT_BYTES as u64);
    assert_eq!(
        stream.current.bytes
            + stream.previous.as_ref().unwrap().bytes
            + stream.evicted_bytes.unwrap(),
        (own + child_total + 1) * (record().len() as u64 + 1)
    );
    let exported = export(root.path()).unwrap();
    let records = exported["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| file["records"].as_array().unwrap())
        .count();
    assert_eq!(records as u64, retained);
    let diagnostic_bytes: u64 = [
        Stream::ToolEvents.previous(),
        Stream::ToolEvents.name(),
        SUMMARY,
        LOCK,
    ]
    .into_iter()
    .map(|name| std::fs::metadata(root.path().join(name)).unwrap().len())
    .sum();
    assert!(diagnostic_bytes <= (2 * SEGMENT_BYTES + 2 * SUMMARY_BYTES) as u64);
    assert!(!root.path().join(STAGE).exists());
    println!(
        "cross_process_diagnostic_counts parent_initial={EXCLUSIVE_RECORDS} parent_parallel={own_parallel} child={child} retained={retained} evicted={} generation={} diagnostic_bytes={diagnostic_bytes}",
        stream.evicted_records.unwrap(),
        stream.generation.unwrap()
    );
}
