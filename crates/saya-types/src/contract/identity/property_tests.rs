//! Properties 3 & 4 (spec §1): object identity separates profiles, and name
//! validation is total — `DatabaseObjectRef::new` returns a ref whose
//! getters round-trip the input, or a typed error, and never panics. Pure.
use super::*;
use proptest::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// A valid profile identity: the `p-` prefix plus 64 lowercase hex digits.
/// Two of these are distinct unless their hex strings are identical.
fn profile_identity(hex: String) -> ProfileIdentity {
    let full = format!("p-{}", hex);
    ProfileIdentity::parse(&full).expect("valid hex profile identity")
}

/// 64 lowercase hex digits.
fn hex64() -> impl Strategy<Value = String> {
    "[0-9a-f]{64}"
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// Any string a user might hand in as a catalog/schema/object name.
fn name() -> impl Strategy<Value = String> {
    // Include empty, long, control chars, and unicode — every rejection
    // path must fire, and none may panic.
    prop::collection::vec(any::<char>(), 0..=140).prop_map(|chars| chars.into_iter().collect())
}

/// A kind strategy: `select` needs a `Vec` here, an array does not satisfy
/// `Into<Cow<'static, [_]>>`.
fn kind() -> impl Strategy<Value = DatabaseObjectKind> {
    prop::sample::select(vec![DatabaseObjectKind::Table, DatabaseObjectKind::View])
}

proptest! {
    /// Property 3 — for two distinct profile identities and the same
    /// qualified name, the two `DatabaseObjectRef`s are unequal and hash
    /// unequally, so a profile-scoped key set cannot alias them.
    #[test]
    fn object_identity_separates_profiles(
        hex_a in hex64(),
        hex_b in hex64(),
    ) {
        prop_assume!(hex_a != hex_b);
        let pa = profile_identity(hex_a);
        let pb = profile_identity(hex_b);

        let ref_a = DatabaseObjectRef::new(pa, "c", "s", "o", DatabaseObjectKind::Table)
            .expect("valid names");
        let ref_b = DatabaseObjectRef::new(pb, "c", "s", "o", DatabaseObjectKind::Table)
            .expect("valid names");
        // Compare by reference so the values are not moved before hashing.
        prop_assert_ne!(&ref_a, &ref_b);
        let ha = hash_of(&ref_a);
        let hb = hash_of(&ref_b);
        prop_assert_ne!(ha, hb);
    }

    /// Property 4 — `DatabaseObjectRef::new` is total: for any string in any
    /// of the three name slots, it returns Ok with getters that round-trip the
    /// exact input, or a typed error. Never panics.
    #[test]
    fn name_validation_is_total(
        catalog in name(),
        schema in name(),
        object in name(),
        kind in kind(),
    ) {
        let profile = profile_identity("a".repeat(64));
        let result = DatabaseObjectRef::new(profile, &catalog, &schema, &object, kind);
        match result {
            Ok(r) => {
                // Round-trip: the getters return the exact input bytes.
                prop_assert_eq!(r.catalog(), catalog.as_str());
                prop_assert_eq!(r.schema(), schema.as_str());
                prop_assert_eq!(r.object(), object.as_str());
                prop_assert_eq!(r.kind(), kind);
                // A ref that round-tripped cannot contain a control char in a
                // name — validation would have rejected it.
                for s in [r.catalog(), r.schema(), r.object()] {
                    prop_assert!(
                        !s.chars().any(|c| c.is_control()),
                        "control char survived validation: {s:?}"
                    );
                }
                // The qualified name is the three parts joined by `.`.
                prop_assert_eq!(r.qualified_name(), format!("{catalog}.{schema}.{object}"));
            }
            Err(e) => {
                // A typed error, and it must be one of the name errors.
                prop_assert!(
                    matches!(
                        e,
                        ContractError::EmptyName
                            | ContractError::NameTooLong
                            | ContractError::ControlCharacter
                    ),
                    "unexpected error: {e:?}"
                );
            }
        }
    }
}
