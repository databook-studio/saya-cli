use saya_types::{DatabaseProfile, PostgresSslMode};

#[test]
fn postgres_sslmode_round_trips_and_old_profiles_remain_valid() {
    let profile: DatabaseProfile = serde_json::from_str(
        r#"{"type":"postgresql","host":"db.test","port":5432,"database":"app","user":"reader","sslmode":"verify-full"}"#,
    )
    .unwrap();
    assert!(matches!(
        profile,
        DatabaseProfile::Postgres {
            ssl_mode: Some(PostgresSslMode::VerifyFull),
            ..
        }
    ));

    let legacy: DatabaseProfile = serde_json::from_str(
        r#"{"type":"postgresql","host":"db.test","database":"app","user":"reader"}"#,
    )
    .unwrap();
    assert!(matches!(
        legacy,
        DatabaseProfile::Postgres { ssl_mode: None, .. }
    ));
}
