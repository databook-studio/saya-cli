use super::*;

pub(super) fn worktree(label: &str) -> WorktreeId {
    WorktreeId::from_canonical_path(&format!("/tmp/saya-pref-unit/{label}")).unwrap()
}

pub(super) fn fresh_store(label: &str) -> SqliteStateStore {
    let dir = std::env::temp_dir().join(format!(
        "saya-preferences-unit-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    SqliteStateStore::new(dir.join("state.sqlite3"))
}

pub(super) fn added(outcome: &PreferenceOutcome) -> ProjectPreference {
    match outcome {
        PreferenceOutcome::Added(record) => record.clone(),
        other => panic!("expected Added, got {other:?}"),
    }
}

pub(super) fn listed(outcome: &PreferenceOutcome) -> (String, Vec<ProjectPreference>) {
    match outcome {
        PreferenceOutcome::Listed {
            worktree_label,
            items,
        } => (worktree_label.clone(), items.clone()),
        other => panic!("expected Listed, got {other:?}"),
    }
}

pub(super) fn added_or_edited(outcome: &PreferenceOutcome) -> ProjectPreference {
    match outcome {
        PreferenceOutcome::Added(record) | PreferenceOutcome::Edited(record) => record.clone(),
        other => panic!("expected Added or Edited, got {other:?}"),
    }
}
