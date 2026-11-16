use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{SnapshotOmissionPolicy, SnapshotOmissionReason, SnapshotSelection, sensitive_path},
};
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_executable, repository, state_root};

/// Names that merely contain `secret` or `credentials`, and files under a
/// generic `.config`, `.claude`, or `.docker` directory, are ordinary
/// repository content: the narrowed set never omits them.
const ORDINARY: &[&str] = &[
    "secrets_manager.py",
    ".config/app.toml",
    ".claude/settings.json",
    ".docker/config.json",
    "src/credentials.rs",
    "docs/secrets.md",
];

/// Real credential shapes: exact names, the `.env.*` family, and the key/cert
/// extensions. They stay omitted.
const CREDENTIALS: &[&str] = &[
    ".env",
    ".env.local",
    ".netrc",
    ".npmrc",
    ".pypirc",
    "credentials.json",
    "id_rsa",
    "id_ed25519",
    "cert.pem",
    "cert.key",
    "cert.p12",
    "cert.pfx",
];

async fn write_untracked(root: &std::path::Path, paths: &[&str]) {
    for path in paths {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, b"fixture placeholder only\n").unwrap();
    }
}

#[tokio::test]
async fn ordinary_source_and_config_paths_are_snapshotted() {
    let temp = TempRoot::new("snapshot-narrowed-included");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    write_untracked(&root, ORDINARY).await;

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-narrowed-included").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    for path in ORDINARY {
        assert!(
            manifest
                .entries()
                .iter()
                .any(|entry| entry.path().as_str() == *path),
            "{path} is ordinary repository content and must be snapshotted"
        );
        assert!(
            manifest
                .omissions()
                .iter()
                .all(|fact| fact.path().as_str() != *path),
            "{path} must not be omitted"
        );
    }
}

#[tokio::test]
async fn real_credential_names_stay_omitted() {
    let temp = TempRoot::new("snapshot-narrowed-omitted");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    write_untracked(&root, CREDENTIALS).await;

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-narrowed-omitted").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    for path in CREDENTIALS {
        assert!(
            manifest
                .entries()
                .iter()
                .all(|entry| entry.path().as_str() != *path),
            "{path} is a credential shape and must be omitted"
        );
        assert!(
            manifest.omissions().iter().any(|fact| {
                fact.path().as_str() == *path
                    && fact.reason() == SnapshotOmissionReason::SensitiveUntracked
            }),
            "{path} must be omitted with the sensitive reason"
        );
    }
}

/// The in-place predicate used by workspace delete, search, and before-images
/// is the same narrowed set: no duplicate rule table.
#[test]
fn the_in_place_predicate_uses_the_same_narrowed_set() {
    for path in ORDINARY {
        assert!(!sensitive_path(path), "{path} is ordinary content");
    }
    for path in CREDENTIALS {
        assert!(sensitive_path(path), "{path} is a credential shape");
    }
    assert!(
        sensitive_path(".ssh/config"),
        "a credential store directory stays sensitive"
    );
}
