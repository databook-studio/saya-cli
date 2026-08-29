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

// The two shipped example connection files are real inputs — they must
// parse end to end so a typo or a stale key list never ships a config the
// binary itself would reject. `examples/` lives at the workspace root, two
// levels above this crate's manifest.
fn example(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

#[test]
fn shipped_connections_example_parses_end_to_end() {
    let file = ConnectionsFile::from_toml(&example("connections.toml"))
        .expect("examples/connections.toml must parse");
    assert_eq!(file.profiles.len(), 7);
}

#[test]
fn shipped_connections_docker_example_parses_end_to_end() {
    let file = ConnectionsFile::from_toml(&example("connections.docker.toml"))
        .expect("examples/connections.docker.toml must parse");
    assert_eq!(file.profiles.len(), 2);
}

// Invariant: the accepted key set is owned by the type, once. serde's
// `deny_unknown_fields` enforces it directly, so a rejected key's error names
// the type's own fields (`expected one of ... <a declared field> ...`).
// A reintroduced hand-written shadow list would either stop rejecting unknown
// keys (if it replaced serde) or carry a different message (`unknown key ... in
// profile ...`) — in either case this assertion, which pins the serde-shaped
// message and a declared Postgres field, breaks. That breakage is the alarm:
// it means the key set is no longer defined by the type alone.
#[test]
fn rejected_profile_key_error_names_the_types_own_fields() {
    let toml = r#"
[profiles.analytics]
type = "postgresql"
host = "localhost"
database = "db"
user = "u"
sslmodee = "verify-full"
"#;
    let error = ConnectionsFile::from_toml(toml).expect_err("typo'd sslmode must fail");
    let message = error.to_string();
    assert!(
        message.contains("unknown field"),
        "serde must reject the unknown key: {message}"
    );
    assert!(
        message.contains("expected one of"),
        "the rejected set must come from the type, not a shadow list: {message}"
    );
    // `password` is a declared Postgres field. If it disappears from the
    // expected list, the type and the error have drifted apart.
    assert!(
        message.contains("password"),
        "the expected list must reflect the type's fields: {message}"
    );
}
