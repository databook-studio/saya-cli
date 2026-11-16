//! Tests for the warehouse question sets (Snowflake, ClickHouse, BigQuery):
//! scripted prompts, defaults shown, and every secret accepted only as an
//! environment-variable name or a file path — never a value.

use std::io::Cursor;

use saya_types::{DatabaseProfile, SecretRef, SnowflakeAuth};

use super::prompt::Prompter;
use super::prompt_warehouse;

/// Runs one scripted warehouse collection; returns (profile, captured output).
/// A cancelled section surfaces as `None` with the transcript preserved.
fn collect(script: &str, engine: &str) -> (Option<DatabaseProfile>, String) {
    let mut input = Cursor::new(script.to_string());
    let mut output = Vec::new();
    let mut prompter = Prompter::new(&mut input, &mut output);
    let result = prompt_warehouse::collect(&mut prompter, engine).ok();
    (result, String::from_utf8(output).unwrap())
}

#[test]
fn clickhouse_defaults_and_password_env() {
    // host, TLS no, port blank (default shown), database, user, password env.
    let (profile, out) = collect(
        "remote.example\nn\n\n\n\nSAYA_CLICKHOUSE_PASSWORD\n",
        "clickhouse",
    );
    assert_eq!(
        profile,
        Some(DatabaseProfile::ClickHouse {
            host: "remote.example".into(),
            port: None,
            database: None,
            user: None,
            password: Some(SecretRef::Env {
                env: "SAYA_CLICKHOUSE_PASSWORD".into()
            }),
            secure: Some(false),
        })
    );
    assert!(
        out.contains("Port [8123]:"),
        "the plain default is shown: {out}"
    );
}

#[test]
fn clickhouse_secure_shows_the_tls_default_port() {
    let (profile, out) = collect("h\ny\n\nwarehouse\nsaya_ro\n\n", "clickhouse");
    assert_eq!(
        profile,
        Some(DatabaseProfile::ClickHouse {
            host: "h".into(),
            port: None,
            database: Some("warehouse".into()),
            user: Some("saya_ro".into()),
            password: None,
            secure: Some(true),
        })
    );
    assert!(
        out.contains("Port [8443]:"),
        "the TLS default is shown: {out}"
    );
}

/// A raw secret value is not a valid environment-variable name: the prompt
/// rejects it and never stores it.
#[test]
fn clickhouse_rejects_a_secret_value_for_the_password() {
    let (profile, out) = collect("h\nn\n\n\n\nopensesame\nSAYA_CH_PASSWORD\n", "clickhouse");
    assert!(out.contains("must match"), "the rejection is shown: {out}");
    assert!(
        !out.contains("password = \"opensesame\""),
        "the value never lands anywhere: {out}"
    );
    assert_eq!(
        profile,
        Some(DatabaseProfile::ClickHouse {
            host: "h".into(),
            port: None,
            database: None,
            user: None,
            password: Some(SecretRef::Env {
                env: "SAYA_CH_PASSWORD".into()
            }),
            secure: Some(false),
        })
    );
}

#[test]
fn bigquery_prompts_required_and_optional_fields() {
    let (profile, _) = collect(
        "my-project\nwarehouse\nUS\n1000000\n/keys/sa.json\n",
        "bigquery",
    );
    assert_eq!(
        profile,
        Some(DatabaseProfile::BigQuery {
            project: "my-project".into(),
            dataset: Some("warehouse".into()),
            location: Some("US".into()),
            max_bytes_billed: Some(1_000_000),
            service_account_key: SecretRef::File {
                file: "/keys/sa.json".into()
            },
        })
    );
}

#[test]
fn bigquery_key_path_is_required_and_never_a_value() {
    // A blank path is rejected once, then the real path is accepted.
    let (profile, out) = collect("p\n\n\n\n\n/keys/sa.json\n", "bigquery");
    assert!(out.contains("this field is required"), "{out}");
    assert_eq!(
        profile,
        Some(DatabaseProfile::BigQuery {
            project: "p".into(),
            dataset: None,
            location: None,
            max_bytes_billed: None,
            service_account_key: SecretRef::File {
                file: "/keys/sa.json".into()
            },
        })
    );
}

#[test]
fn snowflake_keypair_secrets_are_references() {
    // account, user, auth menu blank (keypair default), key path, passphrase
    // (a raw value is rejected first), then the optional session fields.
    let (profile, out) = collect(
        "org-account\njane\n\n/keys/rsa_key.p8\nhunter2\nSAYA_SF_PASSPHRASE\nWH\nDB\n\n\n",
        "snowflake",
    );
    assert!(
        out.contains("must match"),
        "the passphrase value is rejected: {out}"
    );
    assert_eq!(
        profile,
        Some(DatabaseProfile::Snowflake {
            account: "org-account".into(),
            user: "jane".into(),
            auth_type: SnowflakeAuth::Keypair,
            private_key: Some(SecretRef::File {
                file: "/keys/rsa_key.p8".into()
            }),
            password: None,
            passphrase: Some(SecretRef::Env {
                env: "SAYA_SF_PASSPHRASE".into()
            }),
            warehouse: Some("WH".into()),
            database: Some("DB".into()),
            schema: None,
            role: None,
        })
    );
    assert!(
        out.contains("keypair (recommended)"),
        "the auth menu names the default: {out}"
    );
}

#[test]
fn snowflake_userpass_requires_an_env_name() {
    // Blank is rejected for the required password env, then accepted.
    let (profile, out) = collect("acct\njane\n3\n\nSAYA_SF_PASSWORD\n\n\n\n\n", "snowflake");
    assert!(out.contains("this field is required"), "{out}");
    assert_eq!(
        profile,
        Some(DatabaseProfile::Snowflake {
            account: "acct".into(),
            user: "jane".into(),
            auth_type: SnowflakeAuth::Userpass,
            private_key: None,
            password: Some(SecretRef::Env {
                env: "SAYA_SF_PASSWORD".into()
            }),
            passphrase: None,
            warehouse: None,
            database: None,
            schema: None,
            role: None,
        })
    );
}

#[test]
fn snowflake_externalbrowser_asks_no_secret() {
    let (profile, out) = collect("acct\njane\n2\n\n\n\n\n\n", "snowflake");
    assert_eq!(
        profile,
        Some(DatabaseProfile::Snowflake {
            account: "acct".into(),
            user: "jane".into(),
            auth_type: SnowflakeAuth::Externalbrowser,
            private_key: None,
            password: None,
            passphrase: None,
            warehouse: None,
            database: None,
            schema: None,
            role: None,
        })
    );
    assert!(
        !out.contains("private key") && !out.contains("Passphrase") && !out.contains("Password"),
        "browser SSO never prompts for a secret: {out}"
    );
}
