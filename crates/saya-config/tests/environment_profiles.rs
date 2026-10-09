use saya_config::{ConfigError, ConnectionsFile, ResolutionInput, resolve};
use saya_types::{DatabaseProfile, MySqlSslMode, SecretRef};

#[path = "environment_profiles/duckdb.rs"]
mod duckdb;
#[path = "environment_profiles/mysql_tls.rs"]
mod mysql_tls;
#[path = "environment_profiles/profile_selection.rs"]
mod profile_selection;
#[path = "environment_profiles/snowflake.rs"]
mod snowflake;
#[path = "environment_profiles/sqlite.rs"]
mod sqlite;
