use saya_types::{DatabaseProfile, SecretRef, SnowflakeAuth};

#[test]
fn snowflake_profiles_round_trip_without_secret_values() {
    let profile = DatabaseProfile::Snowflake {
        account: "org-account.us-east-1.aws".into(),
        user: "jane".into(),
        auth_type: SnowflakeAuth::Externalbrowser,
        private_key: None,
        password: None,
        passphrase: None,
        warehouse: Some("ANALYTICS".into()),
        database: Some("PROD".into()),
        schema: Some("PUBLIC".into()),
        role: Some("ANALYST".into()),
    };
    let encoded = serde_json::to_string(&profile).unwrap();
    let decoded: DatabaseProfile = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, profile);
    assert!(encoded.contains("externalbrowser"));
    assert!(!encoded.contains("secret"));
}

#[test]
fn snowflake_secret_references_serialize_as_references_only() {
    let profile = DatabaseProfile::Snowflake {
        account: "acct".into(),
        user: "reader".into(),
        auth_type: SnowflakeAuth::Keypair,
        private_key: Some(SecretRef::Env { env: "KEY".into() }),
        password: None,
        passphrase: Some(SecretRef::File {
            file: "/literal/path".into(),
        }),
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    };
    let value = serde_json::to_value(profile).unwrap();
    assert_eq!(value["private_key"]["env"], "KEY");
    assert_eq!(value["passphrase"]["file"], "/literal/path");
}
