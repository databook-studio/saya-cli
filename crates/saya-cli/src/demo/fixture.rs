use crate::demo::populate;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::{
    fs::{self, OpenOptions},
    io::Write,
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
    let db = db_path(dir);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let staging = dir.join(format!("demo.sqlite3.tmp-{}-{nanos}", std::process::id()));
    stage_private_file(&staging)?;
    let built = populate::create_fixture_db(&staging).await;
    if let Err(error) = built {
        let _ = fs::remove_file(&staging);
        return Err(error);
    }
    sync_file(&staging)?;
    fs::rename(&staging, &db).map_err(|error| format!("publish demo database: {error}"))?;
    Ok(())
}

fn stage_private_file(path: &Path) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("stage demo database {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("stage demo database {}: {error}", path.display()))
}

fn sync_file(path: &Path) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| format!("sync demo database {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync demo database {}: {error}", path.display()))
}

fn write_connections(dir: &Path) -> Result<(), String> {
    let path = connections_path(dir);
    let escaped = toml::Value::String(db_path(dir).display().to_string()).to_string();
    let content =
        format!("[profiles.demo]\ntype = \"sqlite\"\npath = {escaped}\nread_only = true\n");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    file.write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure {}: {error}", path.display()))?;
    }
    Ok(())
}
