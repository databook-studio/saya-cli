//! A connection profile type this release no longer carries is refused with
//! the removal stated, not a bare serde "unknown variant" error.

use saya_config::{ConfigError, ConnectionsFile};

#[test]
fn a_removed_duckdb_profile_names_the_profile_and_the_removal() {
    let error =
        ConnectionsFile::from_toml("[profiles.local]\ntype = \"duckdb\"\npath = \":memory:\"\n")
            .expect_err("a removed profile type must fail loudly");
    assert!(
        matches!(error, ConfigError::Parse(_)),
        "a removed profile type is an ordinary parse failure: {error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("\"local\""),
        "the error names the profile: {message}"
    );
    assert!(
        message.contains("removed"),
        "the error states the removal: {message}"
    );
    assert!(
        message.contains("duckdb CLI"),
        "the error names the replacement: {message}"
    );
}
