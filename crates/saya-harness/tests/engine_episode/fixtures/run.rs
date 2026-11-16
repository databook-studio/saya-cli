use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh root for one `approved_run`, never one shared by a retried attempt.
///
/// The deadline tests repeat an attempt with a longer ceiling, and a repeated
/// attempt must start from empty state. On Windows a directory whose SQLite
/// file is still open cannot be deleted, so reusing the path per label would
/// carry the previous attempt's `runs` row into the next one and `create_run`
/// would report `Conflict`. The sequence number makes each attempt's store its
/// own; the delete stays as best-effort hygiene for a recycled process id.
fn temp_root(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "saya-engine-episode-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A run already standing at `approved` — the state a run is in when its
/// first episode begins.
struct ApprovedRun {
    root: PathBuf,
    run_dir: PathBuf,
    store: Arc<SqliteStateStore>,
    run_id: RunId,
}

impl ApprovedRun {
    fn workspace(&self) -> Workspace {
        let dir = self.root.join("workspace");
        fs::create_dir_all(&dir).unwrap();
        Workspace::open(&dir).unwrap()
    }
}

/// A retried attempt must not inherit the previous attempt's store.
///
/// The deadline tests repeat an attempt when its ceiling expired before the
/// source started. On Windows the previous attempt's database file is still
/// open when `temp_root` tries to delete the directory, so the delete fails,
/// the old `runs` row survives, and the next `create_run` reports `Conflict`.
/// Reusing a path across attempts is what makes that possible; each attempt
/// owns its root instead.
#[tokio::test]
async fn a_retried_attempt_does_not_inherit_the_previous_store() {
    let first = approved_run("retry-isolation").await;
    let second = approved_run("retry-isolation").await;
    assert_ne!(first.root, second.root);
    first.store.close().await;
    second.store.close().await;
    let roots = [first.root.clone(), second.root.clone()];
    drop(first);
    drop(second);
    for root in roots {
        let _ = fs::remove_dir_all(root);
    }
}

async fn approved_run(label: &str) -> ApprovedRun {
    let root = temp_root(label);
    let store = Arc::new(SqliteStateStore::new(root.join("state.sqlite3")));
    let run_id = RunId::parse(&format!("run-{label}")).unwrap();
    store
        .create_run(NewRun {
            id: run_id.clone(),
            capabilities: RunCapabilityFlags::default(),
            budgets: RunBudgets::default(),
        })
        .await
        .unwrap();
    store
        .set_run_status(&run_id, RunStatus::Approved, None)
        .await
        .unwrap();
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    ApprovedRun {
        root,
        run_dir,
        store,
        run_id,
    }
}
