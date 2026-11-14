use super::*;

#[test]
fn relationship_items_travel_without_identity() {
    let doc = document(vec![relationship_item()]);
    doc.validate().expect("a portable relationship validates");
    let json = serde_json::to_string(&doc).expect("serializes");
    for forbidden in ["\"profile\"", "p-"] {
        assert!(
            !json.contains(forbidden),
            "a portable relationship must not contain {forbidden:?}"
        );
    }
    let reparsed =
        ContextDocumentV1::from_json_bytes(json.as_bytes()).expect("portable output reparses");
    assert_eq!(reparsed, doc);

    // The rebuilt claim is the original, resolved against the profile.
    let rebuilt = reparsed.items[0]
        .payload
        .clone()
        .into_claim(resolve_with(profile()))
        .expect("the target resolves");
    let expected = ClaimPayload::relationship(
        DatabaseObjectRef::new(
            profile(),
            "analytics",
            "core",
            "customers",
            DatabaseObjectKind::Table,
        )
        .unwrap(),
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        Cardinality::ManyToOne,
    )
    .unwrap();
    assert_eq!(rebuilt, expected);
}

#[test]
fn join_rule_items_requalify_their_target() {
    let payload = ClaimPayload::join_rule(
        "analytics.core.customers",
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        "orders.customer_id = customers.id",
        None,
    )
    .unwrap();
    let portable = PortablePayload::from_claim(&payload).unwrap();
    match &portable {
        PortablePayload::JoinRule { target, .. } => {
            assert_eq!(target.catalog.as_deref(), Some("analytics"));
            assert_eq!(target.schema.as_deref(), Some("core"));
            assert_eq!(target.name, "customers");
        }
        _ => panic!("a join rule becomes the portable variant"),
    }
    let doc = document(vec![ContextItem {
        object: object("orders"),
        payload: portable,
        origin_note: None,
    }]);
    let json = serde_json::to_string(&doc).expect("serializes");
    assert!(
        !json.contains("p-"),
        "a portable join rule carries no identity"
    );
    let reparsed =
        ContextDocumentV1::from_json_bytes(json.as_bytes()).expect("portable output reparses");
    let rebuilt = reparsed.items[0]
        .payload
        .clone()
        .into_claim(resolve_with(profile()))
        .expect("the target resolves");
    assert_eq!(
        rebuilt, payload,
        "the resolved ref's qualified name restores the original target"
    );
}

#[test]
fn join_rule_targets_without_three_parts_travel_as_a_bare_name() {
    let payload = ClaimPayload::join_rule(
        "public.customers",
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        "orders.customer_id = customers.id",
        None,
    )
    .unwrap();
    let portable = PortablePayload::from_claim(&payload).unwrap();
    match &portable {
        PortablePayload::JoinRule { target, .. } => {
            assert_eq!(target.catalog, None);
            assert_eq!(target.schema, None);
            assert_eq!(target.name, "public.customers");
        }
        _ => panic!("a join rule becomes the portable variant"),
    }
    // The resolver remaps the bare name: the rebuilt target is the resolved
    // ref's qualified name in the destination's naming, never the source's.
    let rebuilt = portable.into_claim(resolve_with(profile())).unwrap();
    match rebuilt {
        ClaimPayload::JoinRule { target, .. } => {
            assert_eq!(target, "analytics.core.public.customers");
        }
        _ => panic!("the join rule rebuilds"),
    }
}

#[test]
fn claim_wrapper_refuses_target_bearing_payloads() {
    let relationship = ClaimPayload::relationship(
        DatabaseObjectRef::new(
            profile(),
            "analytics",
            "core",
            "customers",
            DatabaseObjectKind::Table,
        )
        .unwrap(),
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        Cardinality::ManyToOne,
    )
    .unwrap();
    let wrapped = PortablePayload::Claim {
        claim: relationship.clone(),
    };
    let doc = document(vec![ContextItem {
        object: object("orders"),
        payload: wrapped,
        origin_note: None,
    }]);
    assert_eq!(doc.validate(), Err(ContextError::PayloadNotPortable));
    assert!(matches!(
        doc.to_json_pretty(),
        Err(ContextError::PayloadNotPortable)
    ));
    assert_eq!(
        ContextDocumentV1::from_json_bytes(CLAIM_WRAPPED_RELATIONSHIP_JSON),
        Err(ContextError::PayloadNotPortable)
    );
    assert_eq!(
        ContextDocumentV1::from_json_bytes(CLAIM_WRAPPED_JOIN_RULE_JSON),
        Err(ContextError::PayloadNotPortable)
    );
    // into_claim refuses too, and never consults a resolver for it.
    let rebuilt = PortablePayload::Claim {
        claim: relationship,
    }
    .into_claim(|_| None);
    assert_eq!(rebuilt, Err(ContextError::PayloadNotPortable));
}

#[test]
fn unresolved_targets_are_refused() {
    let rebuilt = document(vec![relationship_item()]).items[0]
        .payload
        .clone()
        .into_claim(|_| None);
    assert_eq!(rebuilt, Err(ContextError::UnresolvedTarget));
    let payload = ClaimPayload::join_rule(
        "analytics.core.customers",
        vec!["customer_id".to_string()],
        vec!["id".to_string()],
        "a = b",
        None,
    )
    .unwrap();
    let rebuilt = PortablePayload::from_claim(&payload)
        .unwrap()
        .into_claim(|_| None);
    assert_eq!(rebuilt, Err(ContextError::UnresolvedTarget));
}
