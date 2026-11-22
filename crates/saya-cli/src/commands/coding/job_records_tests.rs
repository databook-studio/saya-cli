//! The job records owner: the two facts a delete acts on, read from a job's own
//! directory, and the bounded removal that takes exactly that directory.

use super::*;

fn scratch(label: &str) -> PathBuf {
    let base = std::env::current_dir()
        .expect("the test runs in the checkout")
        .join("target")
        .join(format!(
            "tmp-job-records-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("the scratch root is creatable");
    base
}

fn job(id: &str) -> JobId {
    JobId::parse(id).expect("the fixture's job id parses")
}

/// A job directory with one record in it and one nested directory.
fn records(root: &Path, id: &str) -> PathBuf {
    let directory = root.join(id);
    fs::create_dir_all(directory.join("checkpoints")).expect("the records are creatable");
    fs::write(directory.join("run-notes.txt"), "x").expect("a record is writable");
    fs::write(directory.join("checkpoints").join("one"), "y").expect("a record is writable");
    directory
}

#[test]
fn the_removal_takes_the_whole_directory_and_no_sibling() {
    let root = scratch("whole");
    let directory = records(&root, "job-one");
    let sibling = records(&root, "job-two");
    let removed = delete_records(&root, &job("job-one"));
    assert!(removed.is_ok(), "the job's own directory goes: {removed:?}");
    assert!(!directory.exists(), "the job's records are gone");
    assert!(
        !root.join("inplace-leases").exists(),
        "nothing beside the job was created"
    );
    assert_eq!(
        fs::read_to_string(sibling.join("run-notes.txt")).unwrap(),
        "x",
        "a sibling job is untouched"
    );
    let removed = delete_records(&root, &job("job-two"));
    assert!(removed.is_ok(), "a second removal converges: {removed:?}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_absent_job_directory_is_already_removed() {
    let root = scratch("absent");
    let removed = delete_records(&root, &job("job-missing"));
    assert!(
        removed.is_ok(),
        "an absent directory is already gone: {removed:?}"
    );
    assert!(
        !root.join("job-missing").exists(),
        "an absent directory is not created to be removed"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_id_that_is_not_one_name_is_refused() {
    let root = scratch("escape");
    let sibling = records(&root, "job-kept");
    // The ids a removal must never reach: the two directories beside a job's
    // own, and anything that is not one name.
    for id in ["inplace-leases", "snapshots", ".", ".."] {
        let parsed = match JobId::parse(id) {
            Ok(parsed) => parsed,
            // The id alphabet refuses it before this owner ever sees it.
            Err(_) => continue,
        };
        assert!(
            matches!(
                delete_records(&root, &parsed),
                Err(CodingError::StateDirectory)
            ),
            "{id:?}"
        );
    }
    assert!(sibling.exists(), "nothing beside the job was touched");
    assert!(
        !root.join("inplace-leases").exists(),
        "no lease dir appeared"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn a_link_inside_the_records_is_unlinked_and_never_followed() {
    let root = scratch("link");
    let directory = records(&root, "job-linked");
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep"), "kept").unwrap();
    std::os::unix::fs::symlink(&outside, directory.join("link")).unwrap();
    let removed = delete_records(&root, &job("job-linked"));
    assert!(removed.is_ok(), "the linked records go: {removed:?}");
    assert!(!directory.exists(), "the job's records are gone");
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "kept",
        "the link's target is untouched"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_facts_are_the_ones_the_records_state() {
    let root = scratch("facts");
    let directory = records(&root, "job-session");
    let facts = super::facts(&root, &job("job-session"));
    assert_eq!(
        facts,
        JobFacts {
            origin: JobOrigin::Cli,
            terminal: false,
            lease_held: false,
            records_present: true
        },
        "a request that cannot be read is a cli job, and no outcome is not terminal"
    );
    fs::write(
        directory.join("outcome.json"),
        "{\"state\":\"cancelled\",\"answer\":null}",
    )
    .unwrap();
    assert!(
        super::facts(&root, &job("job-session")).terminal,
        "cancelled is terminal"
    );
    fs::write(directory.join("outcome.json"), "{\"state\":\"paused\"}").unwrap();
    assert!(
        !super::facts(&root, &job("job-session")).terminal,
        "a paused job is not terminal"
    );
    assert!(
        !super::facts(&root, &job("job-absent")).records_present,
        "a job with no directory records nothing"
    );
    let _ = fs::remove_dir_all(root);
}
