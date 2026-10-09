use super::*;

#[test]
fn portable_relationship_validates_its_columns() {
    let target = PortableObject {
        catalog: Some("analytics".to_string()),
        schema: Some("core".to_string()),
        name: "customers".to_string(),
        kind: DatabaseObjectKind::Table,
    };
    let payload = |local: Vec<String>, target_columns: Vec<String>| PortablePayload::Relationship {
        target: target.clone(),
        local_columns: local,
        target_columns,
        cardinality: Cardinality::ManyToOne,
    };
    assert_eq!(
        payload(vec!["a".to_string()], vec![]).validate(),
        Err(ContextError::InvalidColumns)
    );
    assert_eq!(
        payload(vec![], vec![]).validate(),
        Err(ContextError::InvalidColumns),
        "relationship columns must be non-empty"
    );
    let many: Vec<String> = (0..33).map(|i| format!("c{i}")).collect();
    assert_eq!(
        payload(many.clone(), many).validate(),
        Err(ContextError::InvalidColumns)
    );
    assert_eq!(
        payload(vec!["a\nb".to_string()], vec!["a".to_string()]).validate(),
        Err(ContextError::ControlCharacter)
    );
    let pair = payload(vec!["customer_id".to_string()], vec!["id".to_string()]);
    pair.validate().expect("a valid column pair passes");
}

#[test]
fn portable_join_rule_validates_its_columns_and_text() {
    let target = PortableObject {
        catalog: None,
        schema: None,
        name: "customers".to_string(),
        kind: DatabaseObjectKind::Table,
    };
    let rule = |condition: &str, reason: Option<&str>| PortablePayload::JoinRule {
        target: target.clone(),
        local_columns: vec![],
        target_columns: vec![],
        condition: condition.to_string(),
        reason: reason.map(str::to_string),
    };
    rule("a = b", None)
        .validate()
        .expect("a join rule may carry no columns at all");
    assert_eq!(
        PortablePayload::JoinRule {
            target: target.clone(),
            local_columns: vec!["a".to_string()],
            target_columns: vec![],
            condition: "a = b".to_string(),
            reason: None,
        }
        .validate(),
        Err(ContextError::InvalidColumns),
        "a half-empty column pair does not pair"
    );
    assert_eq!(rule("", None).validate(), Err(ContextError::InvalidText));
    assert_eq!(
        rule("a\u{7}b", None).validate(),
        Err(ContextError::ControlCharacter)
    );
    assert_eq!(
        rule(&"x".repeat(MAX_TEXT_CHARS + 1), None).validate(),
        Err(ContextError::InvalidText)
    );
    rule(&"x".repeat(MAX_TEXT_CHARS), None)
        .validate()
        .expect("1024 characters fit");
    assert_eq!(
        rule("a = b", Some("r\n")).validate(),
        Err(ContextError::ControlCharacter)
    );
    assert_eq!(
        rule("a = b", Some(&"x".repeat(MAX_TEXT_CHARS + 1))).validate(),
        Err(ContextError::InvalidText)
    );
    // A whitespace-only reason survives the gate; the rebuild trims it away.
    rule("a = b", Some("   "))
        .validate()
        .expect("whitespace-only reason is text-shaped");
    let rebuilt = rule("a = b", Some("   "))
        .into_claim(resolve_with(profile()))
        .expect("the rebuild resolves");
    assert!(
        matches!(rebuilt, ClaimPayload::JoinRule { reason: None, .. }),
        "a whitespace-only reason collapses to None on rebuild"
    );
}
