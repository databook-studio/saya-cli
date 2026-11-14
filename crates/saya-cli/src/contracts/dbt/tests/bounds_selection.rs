#[test]
fn manifest_size_version_and_selected_node_bounds() {
    // Over 32 MiB: refused before a byte is parsed.
    let path = temp_root("dbt-oversize").join("manifest.json");
    File::create(&path)
        .unwrap()
        .set_len(MAX_MANIFEST_BYTES as u64 + 1)
        .unwrap();
    assert!(matches!(
        parse(&path, &[]),
        Err(DbtManifestError::Oversize(_))
    ));

    // An unsupported schema version is refused, naming the version.
    let v13 = manifest_file(
        "dbt-v13",
        r#"{"metadata":{"dbt_schema_version":"https://schemas.getdbt.com/dbt/manifest/v13.json"}}"#,
    );
    let error = parse(&v13, &[]).unwrap_err();
    assert!(matches!(error, DbtManifestError::UnsupportedVersion(_)));
    assert!(
        error.to_string().contains("v13.json"),
        "the error must name the version: {error}"
    );

    // No metadata at all is not a manifest.
    let bare = manifest_file("dbt-bare", r#"{"nodes":{}}"#);
    assert!(matches!(
        parse(&bare, &[]),
        Err(DbtManifestError::NotAManifest)
    ));

    // Malformed JSON is refused.
    let junk = manifest_file("dbt-junk", "not json");
    assert!(matches!(
        parse(&junk, &[]),
        Err(DbtManifestError::Malformed)
    ));

    // One node over the selection bound is refused, not truncated...
    let too_many = nodes_manifest("dbt-too-many", MAX_SELECTED_NODES + 1);
    assert!(matches!(
        parse(&too_many, &[]),
        Err(DbtManifestError::TooManyNodes(5_001))
    ));

    // ...and exactly the bound passes.
    let at_limit = nodes_manifest("dbt-at-limit", MAX_SELECTED_NODES);
    assert!(parse(&at_limit, &[]).is_ok());
}

#[test]
fn reads_manifest_v10_v11_and_v12() {
    for (contents, version, dbt_version) in [
        (V10, DbtVersion::V10, "1.6.0"),
        (V11, DbtVersion::V11, "1.7.3"),
    ] {
        let import = parse_default(&manifest_file("dbt-versions", contents));
        assert_eq!(import.version, version);
        assert_eq!(import.items.len(), 1);
        assert_eq!(import.items[0].object.name, "dim");
        let note = format!("dbt {dbt_version} model.legacy.dim");
        assert_eq!(import.items[0].origin_note.as_deref(), Some(note.as_str()));
    }
    // v12 is exercised throughout the other tests; its version still gates.
    let import = parse_default(&manifest_file("dbt-versions-v12", V12));
    assert_eq!(import.version, DbtVersion::V12);
}

#[test]
fn select_globs_filter_nodes_and_unresolved_targets_are_counted() {
    let path = manifest_file("dbt-select", V12);

    // "ord*" selects only orders. Its ref and source targets are not selected,
    // so both relationships are skipped and counted; the test attached to the
    // unselected hidden model is out of scope, not a failure.
    let import = parse(&path, &["ord*"]).expect("the fixture manifest parses");
    assert_eq!(import.items.len(), 3);
    assert!(
        import
            .items
            .iter()
            .all(|item| item.object.name == "public_orders")
    );
    assert_eq!(
        import.skipped,
        vec![
            (
                "test.jaffle.relationships_orders_customer_id".to_string(),
                "unresolved_target"
            ),
            (
                "test.jaffle.relationships_orders_event_id".to_string(),
                "unresolved_target"
            ),
        ]
    );

    // "?" matches exactly one character.
    let import = parse(&path, &["cust?mers"]).expect("the fixture manifest parses");
    assert_eq!(import.items.len(), 1);
    assert_eq!(import.items[0].object.name, "customers");
    assert!(import.skipped.is_empty());
}
