//! Unknown-key rejection for configuration files.
//!
//! A typo'd key must fail loudly at parse time instead of silently falling
//! back to defaults. The security-critical case: `sslmodee = "verify-full"`
//! in a Postgres profile would otherwise silently drop TLS enforcement.

use saya_config::{ConfigError, ConfigFile, ConnectionsFile};

#[test]
fn config_file_rejects_unknown_keys_with_the_key_named() {
    let error = ConfigFile::from_toml("[run]\nmax_rowz = 5\n").expect_err("typo must fail");
    assert!(
        error.to_string().contains("max_rowz"),
        "error must name the offending key: {error}"
    );
}

#[test]
fn config_file_still_accepts_known_sections_and_keys() {
    let config = ConfigFile::from_toml(
        "[run]\nmax_rows = 5\n[ai]\nmodel = 'm'\nallow_data_sharing = true\n",
    )
    .expect("known keys must parse");
    assert_eq!(config.run.max_rows, Some(5));
}

#[test]
fn connections_rejects_unknown_profile_keys_per_backend() {
    let toml = r#"
[profiles.analytics]
type = "postgresql"
host = "localhost"
database = "db"
user = "u"
sslmodee = "verify-full"
"#;
    let error = ConnectionsFile::from_toml(toml).expect_err("typo'd sslmode must fail");
    assert!(
        error.to_string().contains("sslmodee"),
        "error must name the offending key: {error}"
    );
}

#[test]
fn connections_rejects_unknown_top_level_keys() {
    let error = ConnectionsFile::from_toml("[profils.x]\ntype = \"sqlite\"\npath = 'a.db'\n")
        .expect_err("unknown top-level table must fail");
    assert!(
        error.to_string().contains("profils"),
        "error must name the unknown section: {error}"
    );
}

#[test]
fn connections_still_accepts_every_documented_backend_key() {
    let toml = r#"
[profiles.pg]
type = "postgresql"
host = "h"
port = 5432
database = "d"
user = "u"
ssl_mode = "require"
password = { env = "PGPASS" }

[profiles.my]
type = "mysql"
host = "h"
database = "d"
user = "u"
ssl_ca = { file = "/tmp/ca.pem" }

[profiles.duck]
type = "duckdb"
path = ":memory:"
read_only = true

[profiles.lite]
type = "sqlite"
path = "a.db"

[profiles.sf]
type = "snowflake"
account = "acme"
user = "u"
auth_type = "keypair"
private_key = { env = "PK" }
warehouse = "wh"
database = "db"
schema = "public"
role = "r"
"#;
    let profiles = ConnectionsFile::from_toml(toml).expect("documented keys must parse");
    assert_eq!(profiles.profiles.len(), 5);
}

#[test]
fn parse_errors_are_typed_as_config_parse_failures() {
    let error = ConfigFile::from_toml("[run]\nnope = 1\n").unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
}
