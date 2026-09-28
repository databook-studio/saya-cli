use crate::demo::populate;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DB_FILE: &str = "demo.sqlite3";
const CONNECTIONS_FILE: &str = "connections.toml";
pub(crate) const FIXTURE_VERSION: &str = "1";

pub(crate) enum FixtureOutcome {
    Reused,
    Built,
}

pub(crate) fn demo_dir() -> PathBuf {
    let dir = match std::env::var_os("SAYA_DEMO_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => crate::state_path::state_db_path()
            .parent()
            .map(|parent| parent.join("demo"))
            .unwrap_or_else(|| PathBuf::from("saya/demo")),
    };
    absolute(&dir)
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(crate) fn db_path(dir: &Path) -> PathBuf {
    dir.join(DB_FILE)
}

pub(crate) fn connections_path(dir: &Path) -> PathBuf {
    dir.join(CONNECTIONS_FILE)
}

pub(crate) async fn ensure(dir: &Path, reset: bool) -> Result<FixtureOutcome, String> {
    prepare_dir(dir)?;
    if !reset && stored_version(&db_path(dir)).await.as_deref() == Some(FIXTURE_VERSION) {
        write_connections(dir)?;
        return Ok(FixtureOutcome::Reused);
    }
    build_atomically(dir).await?;
    write_connections(dir)?;
    Ok(FixtureOutcome::Built)
}

fn prepare_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir)
        .map_err(|error| format!("create demo directory {}: {error}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure demo directory {}: {error}", dir.display()))?;
    }
    Ok(())
}

async fn stored_version(path: &Path) -> Option<String> {
    let options = SqliteConnectOptions::new().filename(path).read_only(true);
    let pool = SqlitePool::connect_with(options).await.ok()?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM saya_demo_meta WHERE key = 'fixture_version'")
            .fetch_optional(&pool)
            .await
            .ok()?;
    pool.close().await;
    row.map(|(value,)| value)
}

async fn build_atomically(dir: &Path) -> Result<(), String> {
    let staging = staged_sibling(&db_path(dir));
    stage_private_file(&staging)?;
    let built = populate::create_fixture_db(&staging).await;
    if let Err(error) = built {
        let _ = fs::remove_file(&staging);
        return Err(error);
    }
    sync_file(&staging).map_err(|error| format!("sync {}: {error}", staging.display()))?;
    fs::rename(&staging, db_path(dir)).map_err(|error| format!("publish demo database: {error}"))
}

/// Stages a brand-new private sibling file at `path` (create_new, 0600); never overwrites.
fn stage_private_file(path: &Path) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("stage {}: {error}", path.display())),
    }
}

fn sync_file(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.sync_all()
}

/// A hidden, collision-free sibling staging path for `path`.
fn staged_sibling(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    path.with_file_name(format!(".{name}.tmp-{}-{nanos}", std::process::id()))
}

/// Atomic publish of `content` at `path`: a private staged sibling (0600),
/// fsync, rename — refusing (never replacing) a symlink or directory target.
fn publish_private(path: &Path, content: &str) -> Result<(), String> {
    let shown = path.display().to_string();
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_symlink() || meta.is_dir()) {
        return Err(format!(
            "refusing to replace the symlink or directory {shown}"
        ));
    }
    let staging = staged_sibling(path);
    stage_private_file(&staging)?;
    let filled = fs::write(&staging, content).and_then(|()| sync_file(&staging));
    if let Err(error) = filled {
        let _ = fs::remove_file(&staging);
        return Err(format!("write {}: {error}", shown));
    }
    fs::rename(&staging, path).map_err(|error| {
        let _ = fs::remove_file(&staging);
        format!("publish {}: {error}", shown)
    })
}

fn write_connections(dir: &Path) -> Result<(), String> {
    let db = toml::Value::String(db_path(dir).display().to_string()).to_string();
    let content = format!("[profiles.demo]\ntype = \"sqlite\"\npath = {db}\nread_only = true\n");
    publish_private(&connections_path(dir), &content)
}
