use super::*;

/// A record names its run: the job id and the epoch it was read at survive a
/// load and the next save. Written as record text so the test pins the wire
/// shape a job surface will write, not only the Rust field.
#[test]
fn a_record_that_names_its_run_round_trips() {
    let root = temp_root("run-link-roundtrip");
    let store = FsSessionStore::new(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("run-linked.json"),
        r#"{"version":2,"id":"run-linked","provider":"postgres","model":"demo","run":{"job":"job-051","epoch":7},"turns":[],"messages":[]}"#,
    )
    .unwrap();

    let loaded = block_on(store.load("run-linked")).unwrap().unwrap();
    let Some(run) = &loaded.run else {
        panic!("a record that named a run must load with it: {loaded:?}");
    };
    assert_eq!(run.job.as_str(), "job-051", "the job id survives the load");
    assert_eq!(run.epoch, 7, "the epoch survives the load");
    block_on(store.save(loaded)).unwrap();

    let reloaded = block_on(store.load("run-linked")).unwrap().unwrap();
    assert_eq!(
        reloaded.run,
        Some(RunLink {
            job: saya_types::JobId::parse("job-051").unwrap(),
            epoch: 7,
        }),
        "the same link must come back after a save"
    );

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("run-linked.json")).unwrap())
            .unwrap();
    assert_eq!(
        saved["run"]["job"], "job-051",
        "the run's job id must survive a save: {saved}"
    );
    assert_eq!(
        saved["run"]["epoch"], 7,
        "the run's epoch must survive a save: {saved}"
    );
    assert_eq!(saved["version"], 2, "the version stays 2: {saved}");
    let _ = std::fs::remove_dir_all(root);
}

/// A record written before the link existed — version 2, no `run`, no
/// `continuation` — loads, and its next save neither grows the key nor bumps
/// the version: the link is additive, so 0.5.0-shaped records round-trip
/// untouched.
#[test]
fn a_record_written_without_a_run_loads_and_re_saves_without_the_key() {
    let root = temp_root("run-link-absent");
    let store = FsSessionStore::new(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("legacy.json"),
        r#"{"version":2,"id":"legacy","provider":"postgres","model":"demo","approval_mode":"confirm","turns":[],"messages":[]}"#,
    )
    .unwrap();

    let loaded = block_on(store.load("legacy")).unwrap().unwrap();
    assert!(loaded.run.is_none(), "absence stays absence");
    block_on(store.save(loaded)).unwrap();

    let saved = std::fs::read_to_string(root.join("legacy.json")).unwrap();
    assert!(
        !saved.contains("\"run\""),
        "a record that never named a run must not gain the key: {saved}"
    );
    let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(value["version"], 2, "no version bump is owed: {saved}");
    let _ = std::fs::remove_dir_all(root);
}

/// A `run` this binary cannot read refuses the record rather than dropping
/// half a link: the shape is required, and the job id is validated. The
/// strict outcome is the one a malformed `parent` link already takes, so the
/// two lineage links fail alike.
#[test]
fn a_malformed_run_refuses_the_record() {
    let root = temp_root("run-link-malformed");
    let store = FsSessionStore::new(&root);
    std::fs::create_dir_all(&root).unwrap();
    let cases = [
        ("no-epoch", r#"{"job":"job-051"}"#),
        ("string-epoch", r#"{"job":"job-051","epoch":"seven"}"#),
        ("bare", r#""job-051""#),
        ("unknown-job", r#"{"job":"/etc/passwd","epoch":3}"#),
    ];
    for (id, run) in cases {
        std::fs::write(
            root.join(format!("{id}.json")),
            format!(r#"{{"version":2,"id":"{id}","run":{run}}}"#),
        )
        .unwrap();
        assert!(
            block_on(store.load(id)).unwrap().is_none(),
            "{id}: a malformed run must refuse the whole record, not drop the link"
        );
    }

    // The sibling link's own answer today, for comparison: also refused.
    std::fs::write(
        root.join("bad-parent.json"),
        r#"{"version":2,"id":"bad-parent","parent":{"id":"p"}}"#,
    )
    .unwrap();
    assert!(
        block_on(store.load("bad-parent")).unwrap().is_none(),
        "a malformed parent link already refuses the record"
    );
    let _ = std::fs::remove_dir_all(root);
}
