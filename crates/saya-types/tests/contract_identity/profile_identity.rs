//! `ProfileIdentity` parse rules: the `p-` prefix plus exactly 64 lowercase
//! hex digits, and its serde round trip.

use super::*;

#[test]
fn test_1_profile_identity_accepts_valid() {
    let hex = valid_hex();
    let input = format!("p-{hex}");
    let id = ProfileIdentity::parse(&input).unwrap();
    assert_eq!(id.as_str(), &input);
}

#[test]
fn test_1_profile_identity_rejects_too_short() {
    assert!(ProfileIdentity::parse("p-abc").is_err());
}

#[test]
fn test_1_profile_identity_rejects_too_long() {
    let input = format!("p-{}", "a".repeat(65));
    assert!(ProfileIdentity::parse(&input).is_err());
}

#[test]
fn test_1_profile_identity_rejects_missing_prefix() {
    assert!(ProfileIdentity::parse(&valid_hex()).is_err());
}

#[test]
fn test_1_profile_identity_rejects_uppercase_hex() {
    let input = format!("p-{}", "A".repeat(64));
    assert!(ProfileIdentity::parse(&input).is_err());
}

#[test]
fn test_1_profile_identity_rejects_non_hex() {
    let input = format!("p-{}", "g".repeat(64));
    assert!(ProfileIdentity::parse(&input).is_err());
}

#[test]
fn test_2_profile_identity_serde_round_trip() {
    let id = valid_profile();
    let json = serde_json::to_string(&id).unwrap();
    let deserialized: ProfileIdentity = serde_json::from_str(&json).unwrap();
    assert_eq!(id, deserialized);
}

#[test]
fn test_2_profile_identity_serde_rejects_invalid() {
    let result: Result<ProfileIdentity, _> = serde_json::from_str(r#""p-INVALID""#);
    assert!(result.is_err());
}
