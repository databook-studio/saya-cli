use super::*;

#[test]
fn definition_without_parameters_loads_unchanged() {
    let definition = valid_definition();
    let json = definition_json(&definition);
    assert!(
        !json.contains("parameters"),
        "a parameter-free definition omits the field"
    );
    let reparsed = InvestigationDefinitionV1::from_json_bytes(json.as_bytes())
        .expect("old-shaped JSON still loads");
    assert_eq!(reparsed, definition);
    assert!(reparsed.parameters.is_empty());
}

#[test]
fn parameters_round_trip_through_json() {
    let mut definition = definition_with_parameters();
    definition
        .parameters
        .push(parameter_spec("since", ParamType::Date));
    definition
        .parameters
        .iter_mut()
        .find(|spec| spec.name == "since")
        .expect("the date spec exists")
        .required = true;
    let json = definition
        .to_json_pretty()
        .expect("a valid definition serializes");
    assert!(json.contains("\"parameters\""));
    // The wire shape spells the type field "type" per the v1 contract.
    assert!(json.contains("\"type\": \"date\""));
    let reparsed =
        InvestigationDefinitionV1::from_json_bytes(json.as_bytes()).expect("parameters reparses");
    assert_eq!(reparsed, definition);
}

#[test]
fn definition_rejects_oversize_and_unknown_version() {
    let oversized = vec![b' '; MAX_DEFINITION_BYTES + 1];
    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(&oversized),
        Err(InvestigationError::Oversize(MAX_DEFINITION_BYTES + 1))
    );

    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(VERSION_TWO_JSON),
        Err(InvestigationError::UnsupportedVersion(2))
    );

    let definition = valid_definition();
    let reparsed =
        InvestigationDefinitionV1::from_json_bytes(definition_json(&definition).as_bytes())
            .expect("a version-1 definition reparses");
    assert_eq!(reparsed, definition);
    let pretty = definition
        .to_json_pretty()
        .expect("a valid definition serializes");
    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(pretty.as_bytes())
            .expect("pretty output reparses"),
        definition
    );
}

#[test]
fn definition_rejects_unknown_field() {
    let mut json = definition_json(&valid_definition());
    assert!(json.ends_with('}'));
    json.pop();
    json.push_str(", \"sneaky_field\": 1 }");
    assert!(matches!(
        InvestigationDefinitionV1::from_json_bytes(json.as_bytes()),
        Err(InvestigationError::Malformed)
    ));
}

#[test]
fn definition_rejects_wrong_format() {
    assert_eq!(
        InvestigationDefinitionV1::from_json_bytes(WRONG_FORMAT_JSON),
        Err(InvestigationError::NotAnInvestigation)
    );
}

#[test]
fn definition_rejects_missing_version_field() {
    assert!(matches!(
        InvestigationDefinitionV1::from_json_bytes(MISSING_VERSION_JSON),
        Err(InvestigationError::Malformed)
    ));
}

#[test]
fn definition_rejects_an_invalid_id_through_serde() {
    assert!(matches!(
        InvestigationDefinitionV1::from_json_bytes(INVALID_ID_JSON),
        Err(InvestigationError::Malformed)
    ));
}

#[test]
fn serialization_carries_no_secret_shaped_keys() {
    let json = valid_definition()
        .to_json_pretty()
        .expect("a valid definition serializes");
    for forbidden in ["password", "token", "rows", "result", "grant", "identity"] {
        assert!(
            !json.contains(forbidden),
            "serialized definitions must not contain {forbidden:?}"
        );
    }
}
