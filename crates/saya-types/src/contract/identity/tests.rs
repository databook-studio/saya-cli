use super::*;

#[test]
fn parse_valid_profile_identity() {
    let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let input = format!("p-{hex}");
    let id = ProfileIdentity::parse(&input).unwrap();
    assert_eq!(id.as_str(), &input);
}

#[test]
fn parse_rejects_too_short() {
    assert!(ProfileIdentity::parse("p-abc").is_err());
}

#[test]
fn parse_rejects_too_long() {
    let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789ff";
    assert!(ProfileIdentity::parse(&format!("p-{hex}")).is_err());
}

#[test]
fn parse_rejects_missing_prefix() {
    let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    assert!(ProfileIdentity::parse(hex).is_err());
}

#[test]
fn parse_rejects_uppercase_hex() {
    let hex = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
    assert!(ProfileIdentity::parse(&format!("p-{hex}")).is_err());
}

#[test]
fn parse_rejects_non_hex() {
    let hex = "gggggg0123456789gggggg0123456789gggggg0123456789gggggg0123456789";
    assert!(ProfileIdentity::parse(&format!("p-{hex}")).is_err());
}

#[test]
fn display_delegates_to_as_str() {
    let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let input = format!("p-{hex}");
    let id = ProfileIdentity::parse(&input).unwrap();
    assert_eq!(format!("{id}"), input);
}

#[test]
fn serde_round_trip() {
    let hex = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let input = format!("p-{hex}");
    let id = ProfileIdentity::parse(&input).unwrap();
    let json = serde_json::to_string(&id).unwrap();
    let deserialized: ProfileIdentity = serde_json::from_str(&json).unwrap();
    assert_eq!(id, deserialized);
}

#[test]
fn serde_rejects_invalid_string() {
    let result: Result<ProfileIdentity, _> = serde_json::from_str(r#""p-INVALID""#);
    assert!(result.is_err());
}

#[test]
fn database_object_kind_as_str() {
    assert_eq!(DatabaseObjectKind::Table.as_str(), "table");
    assert_eq!(DatabaseObjectKind::View.as_str(), "view");
}

#[test]
fn database_object_kind_parse() {
    assert_eq!(
        DatabaseObjectKind::parse("table"),
        Some(DatabaseObjectKind::Table)
    );
    assert_eq!(
        DatabaseObjectKind::parse("view"),
        Some(DatabaseObjectKind::View)
    );
    assert_eq!(DatabaseObjectKind::parse("unknown"), None);
}

#[test]
fn database_object_ref_new_rejects_empty() {
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let kind = DatabaseObjectKind::Table;
    assert!(DatabaseObjectRef::new(profile.clone(), "", "sch", "obj", kind).is_err());
    assert!(DatabaseObjectRef::new(profile.clone(), "cat", "", "obj", kind).is_err());
    assert!(DatabaseObjectRef::new(profile, "cat", "sch", "", kind).is_err());
}

#[test]
fn database_object_ref_new_rejects_too_long() {
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let long = "a".repeat(129);
    let kind = DatabaseObjectKind::Table;
    assert!(DatabaseObjectRef::new(profile.clone(), &long, "sch", "obj", kind).is_err());
    assert!(DatabaseObjectRef::new(profile.clone(), "cat", &long, "obj", kind).is_err());
    assert!(DatabaseObjectRef::new(profile, "cat", "sch", &long, kind).is_err());
}

#[test]
fn database_object_ref_new_rejects_control_chars() {
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let kind = DatabaseObjectKind::Table;
    assert!(DatabaseObjectRef::new(profile.clone(), "cat\n", "sch", "obj", kind).is_err());
    assert!(DatabaseObjectRef::new(profile.clone(), "cat", "sch\u{0}", "obj", kind).is_err());
    assert!(DatabaseObjectRef::new(profile, "cat", "sch", "obj\r", kind).is_err());
}

#[test]
fn database_object_ref_getters() {
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let r = DatabaseObjectRef::new(
        profile.clone(),
        "cat",
        "sch",
        "obj",
        DatabaseObjectKind::View,
    )
    .unwrap();
    assert_eq!(r.profile(), &profile);
    assert_eq!(r.catalog(), "cat");
    assert_eq!(r.schema(), "sch");
    assert_eq!(r.object(), "obj");
    assert_eq!(r.kind(), DatabaseObjectKind::View);
}

#[test]
fn qualified_name_format() {
    let profile = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let r =
        DatabaseObjectRef::new(profile, "cat", "sch", "obj", DatabaseObjectKind::Table).unwrap();
    assert_eq!(r.qualified_name(), "cat.sch.obj");
}

#[test]
fn cross_profile_isolation() {
    let hex_a = format!("p-{}", "a".repeat(64));
    let hex_b = format!("p-{}", "b".repeat(64));
    let profile_a = ProfileIdentity::parse(&hex_a).unwrap();
    let profile_b = ProfileIdentity::parse(&hex_b).unwrap();
    let ref_a =
        DatabaseObjectRef::new(profile_a, "cat", "sch", "obj", DatabaseObjectKind::Table).unwrap();
    let ref_b =
        DatabaseObjectRef::new(profile_b, "cat", "sch", "obj", DatabaseObjectKind::Table).unwrap();
    assert_ne!(ref_a, ref_b);
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(ref_a.clone());
    assert!(!set.contains(&ref_b));
}
