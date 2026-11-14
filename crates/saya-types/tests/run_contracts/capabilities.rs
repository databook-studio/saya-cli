use super::*;

// ---------------------------------------------------------------------------
// Capabilities: subset semantics and serde
// ---------------------------------------------------------------------------

#[test]
fn capabilities_subset_requires_each_declared_scope_to_be_approved() {
    let approved = approved_scopes();

    let mut workspace = Capabilities::default();
    workspace.workspace_write = true;
    assert!(workspace.is_subset_of(&approved));

    let mut unapproved_workspace = Capabilities::default();
    unapproved_workspace.workspace_write = true;
    assert!(Capabilities::default().is_subset_of(&unapproved_workspace));

    let mut unapproved_workspace_alone = Capabilities::default();
    unapproved_workspace_alone.workspace_write = true;
    assert!(unapproved_workspace_alone.is_subset_of(&approved));
    assert!(!unapproved_workspace_alone.is_subset_of(&Capabilities::default()));

    let mut unapproved_destination = Capabilities::default();
    unapproved_destination.fetch = Some(
        FetchScope::new(vec![Destination::new("https", "internal.example").unwrap()]).unwrap(),
    );
    assert!(!unapproved_destination.is_subset_of(&approved));

    let mut subset_destination = Capabilities::default();
    subset_destination.fetch =
        Some(FetchScope::new(vec![Destination::new("https", "example.com").unwrap()]).unwrap());
    assert!(subset_destination.is_subset_of(&approved));

    let mut unapproved_endpoint = Capabilities::default();
    unapproved_endpoint.endpoints =
        EndpointBindings::new([("target".to_string(), "unbound-endpoint".to_string())]).unwrap();
    assert!(!unapproved_endpoint.is_subset_of(&approved));
}

/// `missing_from` is `is_subset_of`'s message, not a second opinion: it is
/// empty exactly when the subset holds, and every token it names is in the
/// `--allow` grammar the CLI renders refusals with.
#[test]
fn missing_from_is_empty_exactly_when_the_subset_holds_and_names_the_scope() {
    let approved = approved_scopes();
    let mut workspace = Capabilities::default();
    workspace.workspace_write = true;
    assert!(workspace.missing_from(&approved).is_empty());
    assert!(Capabilities::default().missing_from(&approved).is_empty());
    assert!(approved.missing_from(&approved).is_empty());

    let mut unapproved_workspace = Capabilities::default();
    unapproved_workspace.workspace_write = true;
    assert_eq!(
        unapproved_workspace.missing_from(&Capabilities::default()),
        vec!["workspace-write"]
    );

    let mut fetch = Capabilities::default();
    fetch.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "unapproved.example").unwrap(),
        ])
        .unwrap(),
    );
    assert_eq!(
        fetch.missing_from(&approved),
        vec!["fetch:https+unapproved.example"]
    );

    let mut endpoint = Capabilities::default();
    endpoint.endpoints =
        EndpointBindings::new([("target".to_string(), "unbound-endpoint".to_string())]).unwrap();
    assert_eq!(
        endpoint.missing_from(&approved),
        vec!["endpoint:target=unbound-endpoint"]
    );

    // A step asking for several scopes at once names each one that is
    // missing, and nothing that was approved.
    let mut mixed = Capabilities::default();
    mixed.workspace_write = true;
    mixed.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "unapproved.example").unwrap(),
        ])
        .unwrap(),
    );
    let missing = mixed.missing_from(&approved);
    assert_eq!(missing, vec!["fetch:https+unapproved.example"]);
}

/// The property the equivalence name promises, over every shape the subset
/// test exercises: `missing_from` is empty exactly when `is_subset_of` holds.
#[test]
fn missing_from_agrees_with_is_subset_of_on_every_shape() {
    let approved = approved_scopes();
    let shapes: Vec<Capabilities> = vec![
        Capabilities::default(),
        approved.clone(),
        {
            let mut c = Capabilities::default();
            c.workspace_write = true;
            c
        },
        {
            let mut c = Capabilities::default();
            c.fetch = Some(
                FetchScope::new(vec![Destination::new("https", "internal.example").unwrap()])
                    .unwrap(),
            );
            c
        },
        {
            let mut c = Capabilities::default();
            c.fetch = Some(
                FetchScope::new(vec![Destination::new("https", "example.com").unwrap()]).unwrap(),
            );
            c
        },
        {
            let mut c = Capabilities::default();
            c.endpoints =
                EndpointBindings::new([("target".to_string(), "unbound-endpoint".to_string())])
                    .unwrap();
            c
        },
        {
            let mut c = Capabilities::default();
            c.endpoints = endpoint_bindings();
            c
        },
    ];
    for requested in &shapes {
        assert_eq!(
            requested.missing_from(&approved).is_empty(),
            requested.is_subset_of(&approved),
            "missing_from must agree with is_subset_of for {requested:?}"
        );
    }
}

#[test]
fn capabilities_round_trip_through_serde() {
    let json = serde_json::to_string(&approved_scopes()).unwrap();
    let back: Capabilities = serde_json::from_str(&json).unwrap();
    assert_eq!(approved_scopes(), back);
}

/// `Capabilities` has no `deny_unknown_fields`, so a `spec.json`/`plan.json`
/// carrying runner, interpreter or scratch keys still parses: unknown keys
/// are ignored, and `Default` covers their absence. This pins that the
/// earlier wire stays readable.
#[test]
fn capabilities_ignore_removed_runner_interpreter_and_scratch_keys() {
    let old = r#"{
        "write": true,
        "fetch": null,
        "runner": {"programs": ["python3"]},
        "interpreter": {"programs": ["python3"]},
        "scratch": true,
        "endpoints": {}
    }"#;
    let parsed: Capabilities = serde_json::from_str(old).expect("old capabilities still parse");
    let mut expected = Capabilities::default();
    expected.workspace_write = true;
    assert_eq!(parsed, expected);
}

/// The wire name is the short `write` the planner prompt asks for, and the
/// earlier long name stays readable so stored records still load.
#[test]
fn workspace_write_emits_the_short_wire_name_and_still_reads_the_old_one() {
    let short: Capabilities = serde_json::from_str(r#"{"write":true}"#).unwrap();
    assert!(short.workspace_write);
    let old: Capabilities = serde_json::from_str(r#"{"workspace_write":true}"#).unwrap();
    assert!(old.workspace_write);

    let emitted = serde_json::to_value(approved_scopes()).unwrap();
    assert_eq!(emitted["write"], serde_json::json!(true));
    assert!(emitted.get("workspace_write").is_none());
}

#[test]
fn endpoint_bindings_reject_unshaped_names() {
    assert!(EndpointBindings::new([("or chestrator".to_string(), "a".to_string())]).is_err());
    assert!(EndpointBindings::new([("orchestrator".to_string(), String::new())]).is_err());
}
