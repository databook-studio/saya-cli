//! The scratch semantics battery (ADR 0003 §5, M4-1): pins what the bundled
//! DuckDB 1.10505.0 actually does under the narrowed scratch configuration
//! (connector hardening from `duckdb/client.rs:43-52` with external access on)
//! before `scratch_sql` exists. Each test name states a claim; where observed
//! reality differs, the name was amended and the surprise is recorded inline.

use std::{fs, path::Path, path::PathBuf};

use duckdb::{AccessMode, Connection};

#[path = "scratch_semantics/file_creation.rs"]
mod file_creation;

#[path = "scratch_semantics/data_round_trip.rs"]
mod data_round_trip;

#[path = "scratch_semantics/configuration_lock.rs"]
mod configuration_lock;

#[path = "scratch_semantics/extension_egress.rs"]
mod extension_egress;

#[path = "scratch_semantics/external_access.rs"]
mod external_access;

/// The scratch configuration: the connector's hardening verbatim, with
/// `enable_external_access(true)` per ADR 0003 decision 4.
fn scratch_config() -> duckdb::Config {
    duckdb::Config::default()
        .access_mode(AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(true))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .expect("scratch security configuration must build")
}

fn open(path: &Path) -> Connection {
    Connection::open_with_flags(path, scratch_config()).expect("scratch open must succeed")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("saya-scratch-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
