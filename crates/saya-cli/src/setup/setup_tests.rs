//! Tests for the setup engine (S15): draft → plan → commit with private
//! backups and a recoverable interruption marker. The engine is pure; every
//! test drives it against a real temp directory and the real config parsers.

use super::draft::{ProfileDraft, ProviderDraft};
use super::recover::{finish, restore};
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

#[test]
fn setup_cancel_preserves_files() {
    let dir = temp_dir("cancel");
    let config_before = b"default_profile = \"a\"\n\n[run]\nmax_rows = 10\n".to_vec();
    let connections_before = EXISTING_CONNECTIONS.as_bytes().to_vec();
    fs::write(dir.join("config.toml"), &config_before).unwrap();
    fs::write(dir.join("connections.toml"), &connections_before).unwrap();

    let planned = plan(
        &dir,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("c")),
        },
    )
    .unwrap();
    assert_eq!(planned.writes.len(), 1);

    assert_eq!(fs::read(dir.join("config.toml")).unwrap(), config_before);
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        connections_before
    );
    assert!(!dir.join(MARKER_FILE).exists());

    let fresh = fresh_dir("cancel-fresh");
    plan(&fresh, &SetupDraft::default()).unwrap();
    assert!(!fresh.exists(), "plan must not create the directory");

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(fresh);
}

#[test]
fn setup_preserves_existing_profile() {
    let dir = temp_dir("preserve");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();

    let exists = plan(
        &dir,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("a")),
        },
    );
    assert!(matches!(exists, Err(SetupError::ProfileExists(name)) if name == "a"));
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes(),
        "the refused plan still touched nothing"
    );

    let planned = plan(
        &dir,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("c")),
        },
    )
    .unwrap();
    assert_eq!(planned.writes.len(), 1);
    let write = &planned.writes[0];
    assert_eq!(write.file, "connections.toml");
    assert!(!write.created);
    let content = &write.content;
    assert!(
        content.starts_with(EXISTING_CONNECTIONS),
        "existing bytes are an exact prefix"
    );
    let suffix = &content[EXISTING_CONNECTIONS.len()..];
    assert!(
        suffix.starts_with("\n[profiles.c]"),
        "the original ends with a newline, so one blank line then the block"
    );
    let parsed = ConnectionsFile::from_toml(content).unwrap();
    assert!(parsed.profiles.contains_key("a"));
    assert!(parsed.profiles.contains_key("b"));
    assert!(parsed.profiles.contains_key("c"));

    // A file without a trailing newline gets one before the appended block.
    let dir2 = temp_dir("preserve-no-newline");
    let raw = "# only b\n[profiles.b]\ntype = \"duckdb\"\npath = \"/tmp/b.duckdb\"";
    fs::write(dir2.join("connections.toml"), raw).unwrap();
    let planned = plan(
        &dir2,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("c")),
        },
    )
    .unwrap();
    let content = &planned.writes[0].content;
    assert!(content.starts_with(raw));
    assert!(content[raw.len()..].starts_with("\n\n[profiles.c]"));
    assert!(ConnectionsFile::from_toml(content).is_ok());

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir2);
}

#[test]
fn setup_existing_config_is_never_modified() {
    let dir = temp_dir("config-untouched");
    let before =
        b"default_profile = \"analytics\"\n\n[run]\nread_only = true\nmax_rows = 1000\n".to_vec();
    fs::write(dir.join("config.toml"), &before).unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("team")),
    };
    let planned = plan(&dir, &draft).unwrap();
    assert_eq!(planned.writes.len(), 1, "only connections.toml is written");
    assert_eq!(planned.writes[0].file, "connections.toml");
    assert_eq!(planned.notes.len(), 1);
    let note = &planned.notes[0];
    assert!(note.contains("[ai]"));
    assert!(note.contains("provider = \"openai\""));
    assert!(note.contains("api_key = { env = \"OPENAI_API_KEY\" }"));

    let report = commit(&dir, &planned, || Ok(())).unwrap();
    assert_eq!(report.written, vec!["connections.toml".to_string()]);
    assert_eq!(
        fs::read(dir.join("config.toml")).unwrap(),
        before,
        "config.toml bytes unchanged after commit"
    );
    assert!(dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}

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

#[test]
fn setup_recovers_interrupted_pair_write() {
    let original = EXISTING_CONNECTIONS;
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );

    let dir = temp_dir("recover-restore");
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted_commit(&dir, original, &rewritten);

    let found = pending(&dir).unwrap().expect("the marker is found");
    assert_eq!(found.started_unix_ms, 42);
    assert_eq!(found.entries.len(), 2);

    restore(&dir, &found).unwrap();
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        original.as_bytes()
    );
    assert!(
        !dir.join("config.toml").exists(),
        "the created file is removed"
    );
    assert!(!dir.join(MARKER_FILE).exists());
    assert!(!dir.join(BACKUP_DIR).exists());
    assert!(pending(&dir).unwrap().is_none(), "no marker left");
    restore(&dir, &found).unwrap(); // idempotent: a second call is a no-op
    let _ = fs::remove_dir_all(dir);

    let dir = temp_dir("recover-finish");
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted_commit(&dir, original, &rewritten);
    let found = pending(&dir).unwrap().unwrap();
    finish(&dir, &found).unwrap();
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        rewritten.as_bytes(),
        "finish keeps the new bytes"
    );
    assert!(!dir.join(MARKER_FILE).exists());
    assert!(!dir.join(BACKUP_DIR).exists());
    finish(&dir, &found).unwrap(); // idempotent
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn reload_failure_restores_originals() {
    let dir = temp_dir("reload-fail");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let error = commit(&dir, &planned, || Err("probe failed".into())).unwrap_err();
    assert!(matches!(error, SetupError::ReloadFailed(message) if message.contains("probe failed")));

    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes()
    );
    assert!(
        !dir.join("config.toml").exists(),
        "the created file is deleted"
    );
    assert!(!dir.join(MARKER_FILE).exists());
    assert!(!dir.join(BACKUP_DIR).exists());
    let _ = fs::remove_dir_all(dir);
}

/// F6: a backup copy that fails before any target is published must not leave
/// the marker behind — the next startup would otherwise warn about an
/// "interrupted setup" that never modified anything. The failure is injected
/// by taking the backup directory's name with a regular file, so the first
/// backup copy cannot be written.
#[test]
fn backup_failure_before_any_publish_removes_the_marker() {
    let dir = temp_dir("backup-fail");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    fs::write(dir.join(BACKUP_DIR), b"not a directory\n").unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let error = commit(&dir, &planned, || {
        panic!("reload must not run when the backups failed")
    })
    .unwrap_err();
    assert!(
        matches!(error, SetupError::Io { .. }),
        "the injected backup failure surfaces: {error:?}"
    );
    assert!(
        !dir.join(MARKER_FILE).exists(),
        "no marker is left: no target was ever published"
    );
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes(),
        "the existing target is unchanged"
    );
    assert!(
        !dir.join("config.toml").exists(),
        "no target was created either"
    );
    let _ = fs::remove_dir_all(dir);
}

/// F7: when the reload fails and the automatic restore ALSO fails, the error
/// must not claim the originals were restored. It says the restore did not
/// complete and that `saya setup` will offer a restore on the next run — which
/// only works if the marker stays.
#[cfg(unix)]
#[test]
fn reload_failure_with_failed_restore_keeps_the_marker() {
    let dir = temp_dir("reload-restore-fail");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    let outside = temp_dir("reload-restore-fail-outside");
    fs::write(outside.join("sentinel.toml"), "sentinel = 1\n").unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let backup = dir.join(BACKUP_DIR).join("connections.toml");
    let error = commit(&dir, &planned, || {
        // Sabotage the restore that would follow the reload failure: the
        // backup becomes a symlink, which restore refuses — and never follows.
        fs::remove_file(&backup).unwrap();
        symlink(outside.join("sentinel.toml"), &backup).unwrap();
        Err("probe failed".into())
    })
    .unwrap_err();
    let text = error.to_string();
    assert!(
        matches!(&error, SetupError::ReloadRestoreFailed { message, restore }
            if message.contains("probe failed") && !restore.is_empty()),
        "a distinct error reports the failed restore: {error:?}"
    );
    assert!(
        !text.contains("were restored"),
        "the error never claims the originals were restored: {text}"
    );
    assert!(
        text.contains("did NOT complete"),
        "the message says the restore did not complete: {text}"
    );
    assert!(
        text.contains("saya setup"),
        "the message names the next-run restore offer: {text}"
    );
    assert!(
        pending(&dir).unwrap().is_some(),
        "the marker remains so the next run really offers the restore"
    );
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        planned.writes[0].content.as_bytes(),
        "the restore did not run: the target keeps the new bytes"
    );
    assert!(
        dir.join("config.toml").exists(),
        "the created file was not removed either"
    );
    assert_eq!(
        fs::read_to_string(outside.join("sentinel.toml")).unwrap(),
        "sentinel = 1\n",
        "the sabotaged backup symlink was never followed"
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn api_key_is_only_an_env_reference() {
    let dir = temp_dir("env-ref");
    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: None,
    };
    let planned = plan(&dir, &draft).unwrap();
    let content = &planned.writes[0].content;
    assert!(content.contains("api_key = { env = \"OPENAI_API_KEY\" }"));
    assert!(!content.contains("api_key = \""), "never an inline value");
    let parsed = ConfigFile::from_toml(content).unwrap();
    assert_eq!(
        parsed.ai.api_key,
        Some(SecretRef::Env {
            env: "OPENAI_API_KEY".into()
        })
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rendered_files_parse_with_real_parsers() {
    let cases = vec![
        (
            "pg",
            DatabaseProfile::Postgres {
                host: "db.example.com".into(),
                port: Some(5432),
                database: "app".into(),
                user: "reader".into(),
                ssl_mode: Some(saya_types::PostgresSslMode::Require),
                password: Some(SecretRef::Env {
                    env: "SAYA_PG_PASSWORD".into(),
                }),
            },
        ),
        (
            "mysql",
            DatabaseProfile::Mysql {
                host: "db.example.com".into(),
                port: None,
                database: "app".into(),
                user: "reader".into(),
                ssl_mode: Some(saya_types::MySqlSslMode::VerifyIdentity),
                ssl_ca: None,
                password: Some(SecretRef::Env {
                    env: "SAYA_MYSQL_PASSWORD".into(),
                }),
            },
        ),
        (
            "sqlite",
            DatabaseProfile::Sqlite {
                path: "/tmp/t.db".into(),
                read_only: true,
            },
        ),
        (
            "duckdb",
            DatabaseProfile::DuckDb {
                path: "/tmp/t.duckdb".into(),
                read_only: Some(false),
            },
        ),
    ];
    for (name, profile) in cases {
        let dir = temp_dir(&format!("render-{name}"));
        let draft = SetupDraft {
            provider: None,
            profile: Some(ProfileDraft {
                name: name.into(),
                profile: profile.clone(),
            }),
        };
        let planned = plan(&dir, &draft).unwrap();
        let content = &planned.writes[0].content;
        let parsed =
            ConnectionsFile::from_toml(content).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            parsed.profiles.get(name),
            Some(&profile),
            "round-trips the typed profile"
        );
        assert!(content.starts_with(&format!("[profiles.{name}]")));
        let _ = fs::remove_dir_all(dir);
    }

    // A dotted profile name must survive the round trip (quoted header).
    let dir = temp_dir("render-dotted");
    let draft = SetupDraft {
        provider: None,
        profile: Some(profile_draft("team.laptop")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let parsed = ConnectionsFile::from_toml(&planned.writes[0].content).unwrap();
    assert!(parsed.profiles.contains_key("team.laptop"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn unsupported_engine_is_explicit() {
    let cases = vec![
        (
            "snowflake",
            DatabaseProfile::Snowflake {
                account: "org-account".into(),
                user: "reader".into(),
                auth_type: saya_types::SnowflakeAuth::Keypair,
                private_key: Some(SecretRef::Env {
                    env: "SAYA_SF_KEY".into(),
                }),
                password: None,
                passphrase: None,
                warehouse: None,
                database: None,
                schema: None,
                role: None,
            },
        ),
        (
            "clickhouse",
            DatabaseProfile::ClickHouse {
                host: "ch.example.com".into(),
                port: None,
                database: None,
                user: None,
                password: None,
                secure: None,
            },
        ),
        (
            "bigquery",
            DatabaseProfile::BigQuery {
                project: "my-project".into(),
                dataset: None,
                location: None,
                max_bytes_billed: None,
                service_account_key: SecretRef::Env {
                    env: "SAYA_BQ_KEY".into(),
                },
            },
        ),
    ];
    for (engine, profile) in cases {
        let dir = temp_dir(&format!("unsupported-{engine}"));
        let draft = SetupDraft {
            provider: None,
            profile: Some(ProfileDraft {
                name: "x".into(),
                profile,
            }),
        };
        let error = plan(&dir, &draft).unwrap_err();
        assert!(
            matches!(&error, SetupError::UnsupportedEngine(message)
                if message == &format!("configure {engine} in connections.toml; see docs/connections.md")),
            "unexpected error for {engine}: {error:?}"
        );
        assert!(
            !dir.join("connections.toml").exists(),
            "plan performs no writes"
        );
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(unix)]
#[test]
fn symlink_refusal() {
    let dir = temp_dir("symlink");
    let outside = temp_dir("symlink-outside");
    fs::write(outside.join("elsewhere.toml"), "x = 1\n").unwrap();
    symlink(outside.join("elsewhere.toml"), dir.join("config.toml")).unwrap();
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let error = plan(&dir, &draft).unwrap_err();
    assert!(matches!(error, SetupError::Symlink { .. }));

    // A symlinked marker is refused too, and nothing is written.
    let dir2 = temp_dir("symlink-marker");
    fs::write(dir2.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    symlink(outside.join("elsewhere.toml"), dir2.join(MARKER_FILE)).unwrap();
    let planned = plan(&dir2, &draft).unwrap();
    let error = commit(&dir2, &planned, || Ok(())).unwrap_err();
    assert!(matches!(error, SetupError::Symlink { .. }));
    assert_eq!(
        fs::read(dir2.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes(),
        "nothing was written"
    );

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir2);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn draft_validation_rejects_bad_names() {
    use super::draft::{validate_env_name, validate_profile_name};

    assert!(validate_env_name("OPENAI_API_KEY").is_ok());
    assert!(validate_env_name("_OK").is_ok());
    assert!(validate_env_name("lowercase").is_err());
    assert!(validate_env_name("1STARTS_DIGIT").is_err());
    assert!(validate_env_name("").is_err());
    assert!(validate_env_name(&"A".repeat(65)).is_err());
    assert!(validate_env_name(&"A".repeat(64)).is_ok());

    assert!(validate_profile_name("team.db-1").is_ok());
    assert!(validate_profile_name("bad name").is_err());
    assert!(validate_profile_name("").is_err());
    assert!(validate_profile_name(&"a".repeat(65)).is_err());
    assert!(validate_profile_name(&"a".repeat(64)).is_ok());

    let dir = fresh_dir("bad-draft");
    let draft = SetupDraft {
        provider: Some(ProviderDraft {
            provider: AiProvider::Openai,
            model: "m".into(),
            base_url: None,
            api_key_env: Some("oops".into()),
        }),
        profile: None,
    };
    assert!(matches!(plan(&dir, &draft), Err(SetupError::Draft(_))));
    assert!(!dir.exists(), "plan validates before touching anything");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn empty_draft_commit_is_noop() {
    let dir = fresh_dir("noop");
    let planned = plan(&dir, &SetupDraft::default()).unwrap();
    assert!(planned.writes.is_empty());
    assert!(planned.notes.is_empty());

    let reload_called = std::cell::Cell::new(false);
    let report = commit(&dir, &planned, || {
        reload_called.set(true);
        Ok(())
    })
    .unwrap();
    assert!(report.written.is_empty());
    assert!(
        !reload_called.get(),
        "a zero-write plan is a no-op: no reload"
    );
    assert!(!dir.exists(), "a zero-write plan creates nothing");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn corrupt_or_unknown_marker_is_reported() {
    let dir = temp_dir("marker-corrupt");
    fs::write(dir.join(MARKER_FILE), b"not json").unwrap();
    assert!(matches!(pending(&dir), Err(SetupError::Marker(_))));

    let dir = temp_dir("marker-version");
    let marker = serde_json::json!({ "version": 2, "started_unix_ms": 1, "entries": [] });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
    assert!(matches!(pending(&dir), Err(SetupError::Marker(_))));

    let dir = fresh_dir("marker-absent");
    assert!(pending(&dir).unwrap().is_none());
    let _ = fs::remove_dir_all(dir);
    for label in ["marker-corrupt", "marker-version"] {
        let _ = fs::remove_dir_all(temp_dir(label));
    }
}

#[test]
fn commit_refuses_when_commit_pending() {
    let dir = temp_dir("commit-pending");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    stage_interrupted_commit(
        &dir,
        EXISTING_CONNECTIONS,
        "[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n",
    );
    let draft = SetupDraft {
        provider: None,
        profile: Some(profile_draft("d")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let error = commit(&dir, &planned, || Ok(())).unwrap_err();
    assert!(matches!(error, SetupError::CommitPending { .. }));
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        b"[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n",
        "the refused commit touched nothing"
    );
    let _ = fs::remove_dir_all(dir);
}
