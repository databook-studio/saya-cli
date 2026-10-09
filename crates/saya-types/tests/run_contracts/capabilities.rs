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

    let mut scratch = Capabilities::default();
    scratch.scratch = true;
    assert!(scratch.is_subset_of(&approved));
    assert!(!scratch.is_subset_of(&Capabilities::default()));

    let mut unapproved_destination = Capabilities::default();
    unapproved_destination.fetch = Some(
        FetchScope::new(vec![Destination::new("https", "internal.example").unwrap()]).unwrap(),
    );
    assert!(!unapproved_destination.is_subset_of(&approved));

    let mut subset_destination = Capabilities::default();
    subset_destination.fetch =
        Some(FetchScope::new(vec![Destination::new("https", "example.com").unwrap()]).unwrap());
    assert!(subset_destination.is_subset_of(&approved));

    let mut unapproved_program = Capabilities::default();
    unapproved_program.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
    assert!(!unapproved_program.is_subset_of(&approved));

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

    let mut scratch = Capabilities::default();
    scratch.scratch = true;
    assert_eq!(
        scratch.missing_from(&Capabilities::default()),
        vec!["scratch"]
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

    let mut runner = Capabilities::default();
    runner.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
    assert_eq!(runner.missing_from(&approved), vec!["runner:bash"]);

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
    mixed.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
    let missing = mixed.missing_from(&approved);
    assert_eq!(missing, vec!["runner:bash"]);
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
            c.scratch = true;
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
            c.runner = Some(RunnerScope::new(vec!["bash".to_string()]).unwrap());
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

#[test]
fn endpoint_bindings_reject_unshaped_names() {
    assert!(EndpointBindings::new([("or chestrator".to_string(), "a".to_string())]).is_err());
    assert!(EndpointBindings::new([("orchestrator".to_string(), String::new())]).is_err());
}
