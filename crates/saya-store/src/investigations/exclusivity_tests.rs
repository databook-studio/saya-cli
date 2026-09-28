//! Exclusivity tests for investigation-store writes (A1): concurrent
//! conflicting writers through separate repositories on one root must let
//! exactly one win, the cross-process `.lock` OS lock must respect live
//! holders, and the create publish must never replace a file that appeared
//! after the checks.

use crate::StoreError;
use crate::investigations::{InvestigationRepository, LocalBinding};
use crate::private_file;
use saya_types::{
    SqlDialect,
    investigation::{
        INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1,
        InvestigationId,
    },
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A generous contention budget for the concurrency tests: every critical
/// section is sub-millisecond, so contention burns retry sleeps, never the
/// budget.
const WAIT: Duration = Duration::from_secs(10);

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-investigation-excl-{label}-{}",
        std::process::id()
    ))
}

fn id(name: &str) -> InvestigationId {
    InvestigationId::parse(name).unwrap()
}

fn definition(name_id: &str, revision: u32, tag: &str) -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_owned(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: id(name_id),
        revision,
        name: "Demo investigation".to_owned(),
        description: Some(tag.to_owned()),
        sql: format!("select {revision}"),
        dialect: SqlDialect::Sqlite,
        connection: "warehouse".to_owned(),
        objects: Vec::new(),
        schema_fingerprint: None,
        created_unix_ms: 1_000,
        updated_unix_ms: 1_000,
    }
}

fn binding(name_id: &str) -> LocalBinding {
    LocalBinding {
        version: LocalBinding::VERSION,
        id: id(name_id),
        profile: "warehouse".to_owned(),
        profile_identity: "identity-1".to_owned(),
        reviewed_revision: 2,
        reviewed_schema_fingerprint: Some("schema-fp-1".to_owned()),
        reviewed_unix_ms: 2_000,
    }
}

fn repo(root: &Path) -> InvestigationRepository {
    InvestigationRepository::with_lock_params(root.to_path_buf(), WAIT)
}

fn one_winner_and_conflicts(results: &[Result<(), StoreError>]) -> usize {
    let winners = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(winners, 1, "exactly one conflicting writer must succeed");
    assert!(
        results
            .iter()
            .all(|result| matches!(result, Ok(()) | Err(StoreError::Conflict))),
        "every losing writer must report Conflict, got {results:?}"
    );
    results.iter().position(Result::is_ok).unwrap()
}

#[test]
fn concurrent_creates_with_same_id_let_exactly_one_succeed() {
    let root = temp_root("create-race");
    let _ = std::fs::remove_dir_all(&root);
    let results: Vec<Result<(), StoreError>> = std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|index| {
                let repo = repo(&root);
                let definition = definition("race-create", 1, &format!("writer-{index}"));
                scope.spawn(move || repo.create(&definition))
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    let winner = one_winner_and_conflicts(&results);
    let stored = repo(&root).get(&id("race-create")).unwrap();
    let expected = format!("writer-{winner}");
    assert_eq!(
        stored.description.as_deref(),
        Some(expected.as_str()),
        "the surviving document must be the winner's content"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_updates_from_same_revision_let_exactly_one_succeed() {
    let root = temp_root("update-race");
    let _ = std::fs::remove_dir_all(&root);
    repo(&root)
        .create(&definition("race-update", 1, "seed"))
        .unwrap();
    let results: Vec<Result<(), StoreError>> = std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|index| {
                let repo = repo(&root);
                let definition = definition("race-update", 2, &format!("writer-{index}"));
                scope.spawn(move || repo.update(&definition, 1))
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    let winner = one_winner_and_conflicts(&results);
    let stored = repo(&root).get(&id("race-update")).unwrap();
    let expected = format!("writer-{winner}");
    assert_eq!(stored.revision, 2);
    assert_eq!(
        stored.description.as_deref(),
        Some(expected.as_str()),
        "the surviving document must be the winner's content"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn update_racing_delete_never_resurrects_or_loses_silently() {
    let root = temp_root("update-delete-race");
    let _ = std::fs::remove_dir_all(&root);
    let home = repo(&root);
    for index in 0..4 {
        // A fresh id per round: an update-wins round leaves its document
        // behind (that is the point), so the next round must not reuse it.
        let name = format!("race-del-{index}");
        home.create(&definition(&name, 1, "seed")).unwrap();
        home.put_binding(&binding(&name)).unwrap();
        let updated_definition = definition(&name, 2, "updated");
        let target = id(&name);
        let checked = target.clone();
        let (updated, deleted) = std::thread::scope(|scope| {
            let updater = {
                let repo = repo(&root);
                scope.spawn(move || repo.update(&updated_definition, 1))
            };
            let deleter = {
                let repo = repo(&root);
                scope.spawn(move || repo.delete(&target, 1))
            };
            (updater.join().unwrap(), deleter.join().unwrap())
        });
        match (updated, deleted) {
            (Ok(()), Err(StoreError::Conflict)) => {
                let stored = home.get(&checked).unwrap();
                assert_eq!(
                    stored.revision, 2,
                    "when update wins the document must be the updated revision"
                );
                assert!(
                    home.get_binding(&checked).unwrap().is_some(),
                    "a losing delete must not clear the binding"
                );
            }
            (Err(StoreError::NotFound), Ok(())) => {
                assert_eq!(
                    home.get(&checked),
                    Err(StoreError::NotFound),
                    "when delete wins the document must stay gone"
                );
                assert_eq!(
                    home.get_binding(&checked).unwrap(),
                    None,
                    "a winning delete must clear the binding"
                );
            }
            (updated, deleted) => {
                panic!("unexpected race outcome: update={updated:?} delete={deleted:?}")
            }
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn create_does_not_replace_a_file_that_appears_after_the_check() {
    let root = temp_root("no-replace");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("appeared.json");
    std::fs::write(&target, b"original bytes").unwrap();

    // The target appeared between the caller's existence check and its
    // publish (the hand this test plays for a non-locking writer): the
    // exclusive publish must refuse it, never replace it.
    let error = private_file::stage_and_publish_no_replace(&target, b"new bytes").unwrap_err();
    assert_eq!(error, StoreError::Conflict);
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"original bytes".to_vec(),
        "the no-replace publish must never overwrite the target"
    );
    assert!(
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.path().extension().is_some_and(|ext| ext == "tmp")),
        "no staged temp may be left behind"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn exclusive_publish_installs_bytes_when_target_is_absent() {
    let root = temp_root("no-replace-absent");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("absent.json");
    private_file::stage_and_publish_no_replace(&target, b"new bytes").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new bytes".to_vec());
    assert!(
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.path().extension().is_some_and(|ext| ext == "tmp")),
        "no staged temp may be left behind"
    );
    let _ = std::fs::remove_dir_all(root);
}
