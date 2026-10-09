//! Tests for the setup engine (S15): draft → plan → commit with private
//! backups and a recoverable interruption marker. The engine is pure; every
//! test drives it against a real temp directory and the real config parsers.

use super::draft::{ProfileDraft, ProviderDraft};
use super::recover::finish;
use super::restore::{RestoreOutcome, restore};
use super::{SetupDraft, SetupError, commit, pending, plan};
use saya_config::{AiProvider, ConfigFile, ConnectionsFile};
use saya_types::{DatabaseProfile, SecretRef};
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::symlink;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-setup-unit-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A path that does not exist yet, for asserting that nothing creates it.
fn fresh_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-setup-unit-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn provider_draft() -> ProviderDraft {
    ProviderDraft {
        provider: AiProvider::Openai,
        model: "gpt-4o-mini".into(),
        base_url: Some("https://api.example.com/v1".into()),
        api_key_env: Some("OPENAI_API_KEY".into()),
    }
}

fn profile_draft(name: &str) -> ProfileDraft {
    ProfileDraft {
        name: name.into(),
        profile: DatabaseProfile::Sqlite {
            path: "/tmp/team.db".into(),
            read_only: true,
        },
    }
}

const EXISTING_CONNECTIONS: &str = "# my connections\n\
     [profiles.a]\n\
     type = \"sqlite\"\n\
     path = \"/tmp/a.db\"\n\
     read_only = true\n\
     \n\
     [profiles.b]\n\
     type = \"duckdb\"\n\
     path = \"/tmp/b.duckdb\"\n";

const MARKER_FILE: &str = ".setup-commit.json";
const BACKUP_DIR: &str = ".setup-backup";

include!("setup_tests/setup_workflow.rs");

/// The crash state an interrupted commit leaves: marker written, backups made,
/// the first file already rewritten, the second file never written.
fn stage_interrupted_commit(dir: &std::path::Path, original: &str, rewritten: &str) {
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false },
            { "file": "config.toml", "backup": null, "created": true }
        ]
    });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
    fs::create_dir_all(dir.join(BACKUP_DIR)).unwrap();
    fs::write(dir.join(BACKUP_DIR).join("connections.toml"), original).unwrap();
    fs::write(dir.join("connections.toml"), rewritten).unwrap();
}

include!("setup_tests/interrupted_recovery.rs");

include!("setup_tests/rendering.rs");

include!("setup_tests/validation.rs");

include!("setup_tests/restore_recovery.rs");
