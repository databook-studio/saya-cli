use super::*;

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("saya-witness-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("workspace root must create");
        Self(root)
    }

    fn workspace(&self) -> Workspace {
        Workspace::open(&self.0).expect("workspace must open")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn overlapping_reads_do_not_publish_either_observation() {
    let sandbox = Sandbox::new("overlap");
    let workspace = sandbox.workspace();
    let witnesses = ReadWitnesses::default();
    let first = witnesses.begin_read("notes.md").unwrap();
    let second = witnesses.begin_read("notes.md").unwrap();

    witnesses.finish_read_worker(&first);
    witnesses.finish_read_worker(&second);
    witnesses.observe_read(first, &workspace, "first-digest".into(), 3);
    witnesses.observe_read(second, &workspace, "second-digest".into(), 4);

    assert!(witnesses.snapshot("notes.md", &workspace).is_none());
}

#[test]
fn witness_observations_remain_bounded_at_the_path_limit() {
    let sandbox = Sandbox::new("bounded");
    let workspace = sandbox.workspace();
    let witnesses = ReadWitnesses::default();

    for index in 0..=MAX_WITNESSES {
        let path = format!("file-{index}.txt");
        let ticket = witnesses.begin_read(&path).unwrap();
        witnesses.finish_read_worker(&ticket);
        witnesses.observe_read(ticket, &workspace, format!("digest-{index}"), 1);
    }

    assert_eq!(
        witnesses
            .entries
            .lock()
            .expect("witness map lock must be available")
            .len(),
        MAX_WITNESSES
    );
    let retained = (0..=MAX_WITNESSES)
        .filter(|index| {
            witnesses
                .snapshot(&format!("file-{index}.txt"), &workspace)
                .is_some()
        })
        .count();
    assert_eq!(retained, MAX_WITNESSES);
}
