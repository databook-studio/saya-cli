use saya_types::{DatabaseProfile, MySqlSslMode};

#[test]
fn mysql_tls_modes_round_trip_and_legacy_profiles_keep_no_mode() {
    for mode in [
        "prefer",
        "preferred",
        "require",
        "verify-ca",
        "verify-identity",
    ] {
        let profile: DatabaseProfile = serde_json::from_str(&format!(
            r#"{{"type":"mysql","host":"db","database":"app","user":"reader","sslmode":"{mode}"}}"#,
        ))
        .unwrap();
        assert!(matches!(
            profile,
            DatabaseProfile::Mysql {
                ssl_mode: Some(_),
                ..
            }
        ));
    }
    let legacy: DatabaseProfile =
        serde_json::from_str(r#"{"type":"mysql","host":"db","database":"app","user":"reader"}"#)
            .unwrap();
    assert!(matches!(
        legacy,
        DatabaseProfile::Mysql {
            ssl_mode: None,
            ssl_ca: None,
            ..
        }
    ));
    assert_eq!(MySqlSslMode::Prefer, MySqlSslMode::Prefer);
}
