/// A051: the legacy evidence count is a deliberate wire-compatibility field,
/// not a measurement. The human queue line must never render it (a `0` would
/// imply a count of evidence rows that no longer exist), while the JSON shape
/// keeps `evidence_count: 0` for existing consumers.
#[test]
fn text_queue_omits_the_retired_evidence_count_while_json_keeps_the_legacy_field() {
    let item = ContractQueueItemView {
        profile: "warehouse".into(),
        claim_id: "ki-0123456789abcdef".into(),
        status: "candidate".into(),
        kind: "table_alias".into(),
        value: "customers".into(),
        column: None,
        object: "analytics.public.orders".into(),
        schema_state: "current".into(),
        evidence_count: 0,
        incomplete: false,
        truncated: false,
    };
    let event = TerminalEvent::ContractQueue {
        items: vec![item.clone()],
    };
    let stdout = render_event(&event, RenderFormat::Text).stdout;
    assert!(
        stdout.contains("ki-0123456789abcdef"),
        "the queue line names the claim: {stdout}"
    );
    assert!(
        !stdout.to_lowercase().contains("evidence"),
        "human queue output must omit the retired count: {stdout}"
    );

    let rendered = render_event(&event, RenderFormat::Json);
    let value: Value =
        serde_json::from_str(rendered.stdout.trim_end_matches('\n')).expect("json line is JSON");
    assert_eq!(value["event"], "contract_queue");
    let items = value["items"]
        .as_array()
        .expect("queue event carries items");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["evidence_count"], 0,
        "JSON keeps the documented legacy compatibility field: {items:?}"
    );
    let back: ContractQueueItemView =
        serde_json::from_value(items[0].clone()).expect("queue item deserializes");
    assert_eq!(back, item);
}

#[test]
fn contract_view_json_round_trip() {
    let view = orders_view("current", true);
    let json = serde_json::to_string(&view).expect("serializable");
    let back: ContractView = serde_json::from_str(&json).expect("deserializable");
    assert_eq!(view, back);
}

#[test]
fn contract_view_ndjson_event_round_trip() {
    let contract = orders_view("stale", false);
    let event = TerminalEvent::ContractShow {
        contract: contract.clone(),
    };
    let rendered = render_event(&event, RenderFormat::Ndjson);
    // NDJSON is one JSON object per line; the event serializes to a single line.
    let line = rendered.stdout.trim_end_matches('\n');
    let value: Value = serde_json::from_str(line).expect("ndjson line is JSON");
    // The event tags as "contract_show" (snake_case) and nests the contract.
    assert_eq!(value["event"], "contract_show");
    let back: ContractView =
        serde_json::from_value(value["contract"].clone()).expect("nested contract deserializes");
    assert_eq!(contract, back);
}

#[test]
fn empty_contract_list_is_one_plain_line_not_an_error() {
    let event = TerminalEvent::ContractList { contracts: vec![] };
    let rendered = render_event(&event, RenderFormat::Text);
    assert!(
        rendered.stderr.is_empty(),
        "empty list is not an error: {rendered:?}"
    );
    let stdout = rendered.stdout.trim_end_matches('\n');
    assert!(!stdout.is_empty(), "empty list still says something");
    assert!(
        !stdout.contains('\n'),
        "empty list is a single plain line, got:\n{stdout}"
    );
}

/// The leak this prevents: an opaque profile identity (a hash over host, database,
/// user and scope path) being carried in a serialized contract payload. The
/// `ContractView` type shape must exclude any identity-shaped key — there is no
/// field for it, and the test proves that by construction: the set of keys a
/// fully-populated `ContractView` serializes to is exactly the expected set, with
/// no `identity` / `profile_identity` / `id` key present.
///
/// (Spec §4 test 5 described a runtime "seed every field with `p-<64 hex>` and
/// render" test, then concluded the real assertion is the type-shape one. The
/// runtime half is dropped here — it cannot prove what it claims, since the
/// profile name is legitimately a string and an identity-shaped value in it would
/// be indistinguishable from a name. Only the structural assertion can catch the
/// leak.)
#[test]
fn contract_view_serialized_keys_exclude_opaque_profile_identity() {
    // A fully-populated view so no `skip_serializing_if` field is elided:
    // `truncated: true`, a conflict present, and a claim with `column` and
    // `reason` present (the two optional fields, both `Some`).
    let view = ContractView {
        profile: "warehouse".into(),
        object: "analytics.public.orders".into(),
        schema_state: "current".into(),
        claims: vec![ContractClaimView {
            claim_id: "c-1".into(),
            kind: "column_role".into(),
            origin: "user_explicit".into(),
            status: "confirmed".into(),
            value: "identifier".into(),
            column: Some("id".into()),
            reason: Some("the row's stable identity".into()),
        }],
        conflicts: vec![ContractConflictView {
            kind: "table_grain".into(),
            claim_ids: vec!["c-1".into()],
        }],
        truncated: true,
        incomplete: false,
    };
    let json = serde_json::to_value(&view).expect("serializable to JSON value");
    let object = json
        .as_object()
        .expect("ContractView serializes to a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "claims",
            "conflicts",
            "object",
            "profile",
            "schema_state",
            "truncated"
        ],
        "ContractView must carry no identity-shaped field"
    );

    // A claim's keys too — the only place an identity could hide inside a contract.
    let claim_json = serde_json::to_value(&view.claims[0]).expect("claim serializable");
    let mut claim_keys: Vec<&str> = claim_json
        .as_object()
        .expect("claim serializes to an object")
        .keys()
        .map(String::as_str)
        .collect();
    claim_keys.sort();
    assert_eq!(
        claim_keys,
        [
            "claim_id", "column", "kind", "origin", "reason", "status", "value"
        ],
        "ContractClaimView must carry no identity-shaped field"
    );

    // Compile-time shape check: the DTOs implement both directions, so the wire
    // shape is a real contract, not a one-way print.
    fn _assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}
    _assert_serializable::<ContractView>();
    _assert_serializable::<ContractClaimView>();
    _assert_serializable::<ContractConflictView>();
}
