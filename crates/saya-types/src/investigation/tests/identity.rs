use super::*;
use proptest::prelude::*;

#[test]
fn investigation_id_derives_deterministic_valid_ids() {
    for name in ["  Revenue — Q3 / EU!! ", "", "---"] {
        let first = InvestigationId::derive(name, "select 1", CREATED);
        let second = InvestigationId::derive(name, "select 1", CREATED);
        assert_eq!(first, second, "derive is deterministic for {name:?}");
        InvestigationId::parse(first.as_str())
            .unwrap_or_else(|_| panic!("derived id {:?} must always be valid", first.as_str()));
    }
    assert!(
        InvestigationId::derive("  Revenue — Q3 / EU!! ", "select 1", CREATED)
            .as_str()
            .starts_with("revenue-q3-eu-")
    );
    assert!(
        InvestigationId::derive("", "select 1", CREATED)
            .as_str()
            .starts_with("investigation-")
    );
    assert!(
        InvestigationId::derive("---", "select 1", CREATED)
            .as_str()
            .starts_with("investigation-")
    );

    let long = InvestigationId::derive(&"w".repeat(100), "select 1", CREATED);
    assert_eq!(long.as_str().len(), 48 + 1 + 8);
    InvestigationId::parse(long.as_str()).expect("a capped slug still yields a valid id");

    assert_ne!(
        InvestigationId::derive("a", "select 1", CREATED),
        InvestigationId::derive("a", "select 2", CREATED)
    );
    assert_ne!(
        InvestigationId::derive("a", "select 1", CREATED),
        InvestigationId::derive("a", "select 1", CREATED + 1)
    );
}

#[test]
fn investigation_id_parse_rejects_invalid_shapes() {
    for bad in ["A", "-a", "a-", "a--b", "a/b", ".."] {
        assert_eq!(
            InvestigationId::parse(bad),
            Err(InvestigationError::InvalidId),
            "{bad:?} must be rejected"
        );
    }
    assert_eq!(
        InvestigationId::parse(&"a".repeat(MAX_ID_CHARS + 1)),
        Err(InvestigationError::InvalidId)
    );
    assert_eq!(
        InvestigationId::parse(""),
        Err(InvestigationError::InvalidId)
    );
    assert!(InvestigationId::parse("a").is_ok());
    InvestigationId::parse(&"a".repeat(MAX_ID_CHARS)).expect("64 characters fit");
}

#[test]
fn investigation_id_round_trips_through_serde() {
    let id = InvestigationId::parse("revenue-12345678").expect("a well-shaped id parses");
    assert_eq!(
        serde_json::to_string(&id).expect("ids serialize"),
        "\"revenue-12345678\""
    );
    assert_eq!(
        serde_json::from_str::<InvestigationId>("\"revenue-12345678\"").expect("ids deserialize"),
        id
    );
    assert!(serde_json::from_str::<InvestigationId>("\"Not-Valid\"").is_err());
}

proptest! {
    /// Any name, however hostile, derives an id its own `parse` accepts, and
    /// derivation never wavers for the same inputs.
    #[test]
    fn derived_id_is_always_valid_and_deterministic(
        name in "(?s).{0,120}",
        sql in "(?s).{0,120}",
        created_unix_ms in any::<i64>(),
    ) {
        let first = InvestigationId::derive(&name, &sql, created_unix_ms);
        let second = InvestigationId::derive(&name, &sql, created_unix_ms);
        prop_assert!(InvestigationId::parse(first.as_str()).is_ok());
        prop_assert_eq!(first, second);
    }
}
