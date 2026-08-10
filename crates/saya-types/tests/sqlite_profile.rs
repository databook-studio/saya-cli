use saya_types::{DatabaseProfile, SqlDialect};

#[test]
fn sqlite_profile_deserializes_with_read_only() {
    let profile: DatabaseProfile =
        serde_json::from_str(r#"{"type":"sqlite","path":"/tmp/app.db","read_only":true}"#).unwrap();
    assert_eq!(
        profile,
        DatabaseProfile::Sqlite {
            path: "/tmp/app.db".into(),
            read_only: Some(true),
        }
    );
}

#[test]
fn sqlite_profile_deserializes_omitting_read_only() {
    let profile: DatabaseProfile =
        serde_json::from_str(r#"{"type":"sqlite","path":"/tmp/app.db"}"#).unwrap();
    assert_eq!(
        profile,
        DatabaseProfile::Sqlite {
            path: "/tmp/app.db".into(),
            read_only: None,
        }
    );
}

#[test]
fn sqlite_profile_dialect_and_as_str() {
    let profile = DatabaseProfile::Sqlite {
        path: "/tmp/app.db".into(),
        read_only: Some(false),
    };
    assert_eq!(profile.dialect(), SqlDialect::Sqlite);
    assert_eq!(SqlDialect::Sqlite.as_str(), "sqlite");
}

#[test]
fn sqlite_profile_serde_round_trip() {
    let original = DatabaseProfile::Sqlite {
        path: "/tmp/app.db".into(),
        read_only: Some(true),
    };
    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: DatabaseProfile = serde_json::from_str(&serialized).unwrap();
    assert_eq!(original, deserialized);
}
