//! The sqlite fixture every perf run is configured against: a real file
//! database opened through a read-only profile, so the surface composes the
//! same session a database-aware one does.
//!
//! The database is created through sqlx (not through saya), the profile is a
//! `connections.toml` under the run's own config root, and the config turns
//! memory **off** so one turn is exactly one provider request.

#[path = "../support/sqlite_fixture.rs"]
mod sqlite_fixture;

use std::path::{Path, PathBuf};

use crate::dialect::Dialect;

/// The config every perf run states: sharing is the operator's consent (the
/// scripted mock is not a local endpoint), memory is off so no extraction adds
/// a request to a turn, and the run's own row ceiling is small.
pub(crate) const CONFIG: &str = "[ai]\nallow_data_sharing = true\n\n\
     [memory]\nmode = 'off'\n\n\
     [run]\nmax_rows = 10\n";

/// Creates the fixture database and the `connections.toml` naming it as the
/// read-only profile `local`. Returns that connections file.
pub(crate) fn sqlite_profile(config_root: &Path) -> PathBuf {
    let database = config_root.join("perf.sqlite");
    sqlite_fixture::create(
        &database,
        &[
            "CREATE TABLE revenue (amount INTEGER);",
            "INSERT INTO revenue (amount) VALUES (10), (20);",
        ],
    );
    let connections = config_root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    connections
}

/// The arguments a perf run passes: the fixture profile, the run's own
/// workspace (so the session never binds this repository — the run's roots live
/// under `target/`, inside it), and read-only approval — a perf turn asks no
/// database question, so nothing can prompt.
///
/// The posture is the one setting whose spelling differs between the arms: the
/// control states it as `--approval-mode read-only`, the candidate as
/// `--mode read-only`. `dialect` is the binary's own answer, read from its
/// `--help`, so every arm gets the same setting under the name it knows.
pub(crate) fn args(dialect: &Dialect, connections: &Path, workspace: &Path) -> Vec<String> {
    let posture = dialect.posture.args();
    vec![
        "--connections".to_owned(),
        connections.to_str().unwrap().to_owned(),
        "--profile".to_owned(),
        "local".to_owned(),
        "--workspace".to_owned(),
        workspace.to_str().unwrap().to_owned(),
        posture[0].to_owned(),
        posture[1].to_owned(),
    ]
}
