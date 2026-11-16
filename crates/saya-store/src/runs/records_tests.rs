//! Tests for the run record types' persisted-string gate: the codec's round
//! trip, its refusal of credential-shaped and overwide maps, and the
//! integer-bound helpers.

use super::{i64_of, u64_of};
use std::collections::BTreeMap;

#[test]
fn endpoint_maps_round_trip_through_the_codec() {
    let map = BTreeMap::from([
        ("primary".to_owned(), Some(5u64)),
        ("fallback".to_owned(), None),
    ]);
    let json = super::encode_endpoint_counts(&map).unwrap().unwrap();
    let decoded: BTreeMap<String, Option<u64>> = super::decode_endpoint_counts(Some(json)).unwrap();
    assert_eq!(decoded, map);
    // An empty map is NULL, not `{}`.
    assert!(
        super::encode_endpoint_counts::<u64>(&BTreeMap::new())
            .unwrap()
            .is_none()
    );
    assert!(
        super::decode_endpoint_counts::<u64>(None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn credential_shaped_keys_are_refused_by_the_codec() {
    for bad in [
        "api_key=sk-live-SENTINEL",
        "password=hunter2",
        "postgres://user:SECRET@host/db",
        "/Users/someone/project",
        "Authorization: Bearer x",
        "bad\u{7}key",
        "bad key",
        "",
    ] {
        let map = BTreeMap::from([(bad.to_owned(), 1u64)]);
        assert_eq!(
            super::encode_endpoint_counts(&map),
            Err(crate::StoreError::Invalid),
            "should refuse {bad:?}"
        );
    }
}

#[test]
fn overwide_maps_exceed_the_store_limit() {
    let mut map = BTreeMap::new();
    for index in 0..9 {
        map.insert(format!("endpoint-{index}"), 1u64);
    }
    assert_eq!(
        super::encode_endpoint_counts(&map),
        Err(crate::StoreError::LimitExceeded)
    );
}

#[test]
fn integer_conversion_refuses_what_it_cannot_represent() {
    assert_eq!(i64_of(Some(5)), Ok(Some(5)));
    assert_eq!(i64_of(None), Ok(None));
    assert_eq!(
        i64_of(Some(u64::MAX)),
        Err(crate::StoreError::LimitExceeded)
    );
    assert_eq!(u64_of(Some(-1)), Err(crate::StoreError::Invalid));
    assert_eq!(u64_of(Some(5)), Ok(Some(5)));
    assert_eq!(u64_of(None), Ok(None));
}
