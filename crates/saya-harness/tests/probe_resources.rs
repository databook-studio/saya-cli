//! Isolated actual policy canaries, actual root entries and actual OS threads.
#![cfg(target_os = "macos")]
#[path = "probe_resources/support.rs"]
mod support;
use saya_harness::runner::sandbox::RunSandbox;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

fn entries(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut result = BTreeMap::new();
    for item in fs::read_dir(root).unwrap() {
        let item = item.unwrap();
        assert!(
            result.len() < 32,
            "fixture observation is explicitly bounded"
        );
        let metadata = fs::symlink_metadata(item.path()).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "unexpected fixture link"
        );
        let bytes = if metadata.is_file() {
            assert!(metadata.len() <= 4096);
            Some(fs::read(item.path()).unwrap())
        } else {
            assert!(metadata.is_dir());
            assert!(fs::read_dir(item.path()).unwrap().next().is_none());
            None
        };
        result.insert(item.file_name().into_string().unwrap(), bytes);
    }
    result
}

#[test]
fn repeated_public_prepare_preserves_root_and_drains_listener_threads() {
    if support::isolated("repeated_public_prepare_preserves_root_and_drains_listener_threads") {
        return;
    }
    let evidence = support::root();
    let root = evidence.join("workspace");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("sentinel.txt"), b"untouched synthetic source\n").unwrap();
    fs::create_dir(root.join("allowed-write-existing-user-entry")).unwrap();
    fs::create_dir(root.join("canary-inside-existing-user-entry")).unwrap();
    let root = root.canonicalize().unwrap();
    let before = entries(&root);
    let threads_before = support::thread_count();
    let sandbox = RunSandbox::new([root.clone()], Vec::<(String, u16)>::new()).unwrap();
    let mut reports = Vec::new();
    for _ in 0..2 {
        let provision = sandbox.prepare(Path::new("/bin")).unwrap();
        reports.push(provision.report().render());
        assert!(
            provision.report().proves_runner(),
            "fixture unavailable: real allow/deny policy was not proven: {}",
            provision.report().render()
        );
        assert!(
            provision
                .report()
                .checks()
                .any(|check| check.name() == "write_allowed_inside_root" && check.passed())
        );
    }
    let after = entries(&root);
    let deadline = Instant::now() + Duration::from_secs(2);
    let threads_after = loop {
        let count = support::thread_count();
        if count <= threads_before || Instant::now() >= deadline {
            break count;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let observations = serde_json::json!({"threads_before":threads_before,"threads_after":threads_after,
        "root_before":before,"root_after":after,"reports":reports});
    fs::write(
        evidence.join("observations.json"),
        serde_json::to_vec_pretty(&observations).unwrap(),
    )
    .unwrap();
    let root_clean = before == after;
    let threads_clean = threads_after <= threads_before;
    assert!(
        root_clean && threads_clean,
        "actual prepare leaked owned resources: root_clean={root_clean}, threads_clean={threads_clean}; observed before={threads_before}, after={threads_after}; {observations}"
    );
}
