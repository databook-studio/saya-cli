//! `DatabaseObjectRef` validation: empty, over-long, and control-character
//! names are refused, and the profile isolates identity across sets.

use super::*;

#[test]
fn test_3_object_ref_rejects_empty_name() {
    let profile = valid_profile();
    assert!(DatabaseObjectRef::new(profile, "", "s", "o", DatabaseObjectKind::Table).is_err());
}

#[test]
fn test_3_object_ref_rejects_long_name() {
    let profile = valid_profile();
    let long = "a".repeat(129);
    assert!(DatabaseObjectRef::new(profile, &long, "s", "o", DatabaseObjectKind::Table).is_err());
}

#[test]
fn test_3_object_ref_rejects_control_chars() {
    let profile = valid_profile();
    assert!(
        DatabaseObjectRef::new(profile.clone(), "c\n", "s", "o", DatabaseObjectKind::Table)
            .is_err()
    );
    assert!(
        DatabaseObjectRef::new(profile, "c", "s", "o\u{0}", DatabaseObjectKind::Table).is_err()
    );
}

#[test]
fn test_4_cross_profile_isolation() {
    let profile_a = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let profile_b = ProfileIdentity::parse(&format!("p-{}", "b".repeat(64))).unwrap();
    let ref_a =
        DatabaseObjectRef::new(profile_a, "c", "s", "o", DatabaseObjectKind::Table).unwrap();
    let ref_b =
        DatabaseObjectRef::new(profile_b, "c", "s", "o", DatabaseObjectKind::Table).unwrap();
    assert_ne!(ref_a, ref_b);
    let mut set = HashSet::new();
    set.insert(ref_a);
    assert!(!set.contains(&ref_b));
}
