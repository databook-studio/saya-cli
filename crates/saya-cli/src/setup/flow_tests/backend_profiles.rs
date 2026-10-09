// ------------------------------------------------------------ warehouses ----

/// A ClickHouse flow end to end: TLS yes, defaults taken for port, database
/// and user skipped, the password only as an environment-variable name.
#[test]
fn clickhouse_flow_writes_the_expected_profile() {
    let dir = temp_dir("flow-clickhouse");
    let script = "6\n6\nlocalhost\ny\n\n\n\nSAYA_CH_PASSWORD\nch\ny\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap_or_else(|error| panic!("the written file parses: {error}"));
    assert_eq!(
        parsed.profiles.get("ch"),
        Some(&DatabaseProfile::ClickHouse {
            host: "localhost".into(),
            port: None,
            database: None,
            user: None,
            password: Some(SecretRef::Env {
                env: "SAYA_CH_PASSWORD".into()
            }),
            secure: Some(true),
        }),
        "the scripted flow wrote the expected profile"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A BigQuery flow end to end: required project and key path, optionals
/// skipped. The key is stored as a file reference only.
#[test]
fn bigquery_flow_writes_the_expected_profile() {
    let dir = temp_dir("flow-bigquery");
    let script = "6\n7\nmy-project\n\n\n\n/keys/sa.json\nbq\ny\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap_or_else(|error| panic!("the written file parses: {error}"));
    assert_eq!(
        parsed.profiles.get("bq"),
        Some(&DatabaseProfile::BigQuery {
            project: "my-project".into(),
            dataset: None,
            location: None,
            max_bytes_billed: None,
            service_account_key: SecretRef::File {
                file: "/keys/sa.json".into()
            },
        }),
        "the scripted flow wrote the expected profile"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Declining the browser-SSO consent skips the probe entirely (no probe call,
/// no probing line) and labels it "not probed"; the flow still completes.
#[test]
fn snowflake_sso_probe_declined_skips_and_labels_not_probed() {
    let dir = temp_dir("sso-declined");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let script = "6\n5\nacct\njane\n2\n\n\n\n\n\nn\ny\n";
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("not probed"), "the skip is labelled: {out}");
    let recorded = calls.lock().unwrap().clone();
    assert!(
        !recorded.iter().any(|call| call == "database:snowflake"),
        "declining skips the probe: {recorded:?}"
    );
    assert!(
        !out.contains("Probing the database"),
        "no probing line when skipped: {out}"
    );
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap();
    assert_eq!(
        parsed.profiles.get("snowflake"),
        Some(&DatabaseProfile::Snowflake {
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
    let _ = fs::remove_dir_all(dir);
}

/// Consenting runs the probe with the 120-second SSO window named up front.
#[test]
fn snowflake_sso_probe_consented_runs_with_the_120s_window() {
    let dir = temp_dir("sso-consented");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let script = "6\n5\nacct\njane\n2\n\n\n\n\n\ny\ny\n";
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    let recorded = calls.lock().unwrap().clone();
    assert!(
        recorded.iter().any(|call| call == "database:snowflake"),
        "consent runs the probe: {recorded:?}"
    );
    assert!(
        out.contains("Probing the database (up to 120 seconds)"),
        "the SSO window is announced: {out}"
    );
    assert!(out.contains("database reachable"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

/// Only Snowflake browser-SSO needs the consent gate and the 120-second
/// window; keypair, userpass, and every other engine keep the 15-second one.
#[test]
fn sso_window_and_consent_apply_only_to_browser_auth() {
    let browser = DatabaseProfile::Snowflake {
        account: "a".into(),
        user: "u".into(),
        auth_type: SnowflakeAuth::Externalbrowser,
        private_key: None,
        password: None,
        passphrase: None,
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    };
    let keypair = DatabaseProfile::Snowflake {
        account: "a".into(),
        user: "u".into(),
        auth_type: SnowflakeAuth::Keypair,
        private_key: None,
        password: None,
        passphrase: None,
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    };
    let clickhouse = DatabaseProfile::ClickHouse {
        host: "h".into(),
        port: None,
        database: None,
        user: None,
        password: None,
        secure: None,
    };
    assert!(needs_sso_consent(&browser));
    assert!(!needs_sso_consent(&keypair));
    assert!(!needs_sso_consent(&clickhouse));
    assert_eq!(probe_timeout_for(&browser), SSO_PROBE_TIMEOUT);
    assert_eq!(probe_timeout_for(&keypair), super::probe::PROBE_TIMEOUT);
    assert_eq!(probe_timeout_for(&clickhouse), super::probe::PROBE_TIMEOUT);
    assert_eq!(SSO_PROBE_TIMEOUT, std::time::Duration::from_secs(120));
}
