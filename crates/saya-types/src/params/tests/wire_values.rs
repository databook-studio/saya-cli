use super::*;

#[test]
fn parameter_spec_json_wire_shape() {
    let spec = ParameterSpec::new("region", ParamType::String, true, None).expect("a valid spec");
    let json = serde_json::to_string(&spec).expect("a spec serializes");
    assert_eq!(json, r#"{"name":"region","type":"string","required":true}"#);
    let back: ParameterSpec = serde_json::from_str(&json).expect("the wire shape reparses");
    assert_eq!(back, spec);
    // An optional description is omitted, and a present one round-trips.
    let described = ParameterSpec::new("d", ParamType::Date, false, Some("when".to_owned()))
        .expect("a valid spec");
    let json = serde_json::to_string(&described).expect("a spec serializes");
    assert_eq!(
        json,
        r#"{"name":"d","type":"date","required":false,"description":"when"}"#
    );
}

#[test]
fn parameter_spec_rejects_unknown_fields() {
    let json = r#"{"name":"region","type":"string","required":true,"extra":1}"#;
    assert!(serde_json::from_str::<ParameterSpec>(json).is_err());
    // The Rust field name is not the wire name: "param_type" is refused.
    let json = r#"{"name":"region","param_type":"string","required":true}"#;
    assert!(serde_json::from_str::<ParameterSpec>(json).is_err());
}

#[test]
fn param_type_serializes_snake_case() {
    for (param_type, tag) in [
        (ParamType::String, "string"),
        (ParamType::Integer, "integer"),
        (ParamType::Boolean, "boolean"),
        (ParamType::Decimal, "decimal"),
        (ParamType::Date, "date"),
        (ParamType::Timestamp, "timestamp"),
    ] {
        let json = serde_json::to_string(&param_type).expect("a type serializes");
        assert_eq!(json, format!("\"{tag}\""));
        assert_eq!(param_type.as_str(), tag);
    }
}

#[test]
fn param_value_json_round_trip() {
    for value in [
        ParamValue::Null(ParamType::String),
        ParamValue::String("x".to_owned()),
        ParamValue::Integer(-5),
        ParamValue::Boolean(true),
        ParamValue::Decimal("1.5".to_owned()),
        ParamValue::Date("2024-02-29".to_owned()),
        ParamValue::Timestamp("2024-02-29T00:00:00Z".to_owned()),
    ] {
        let json = serde_json::to_string(&value).expect("a value serializes");
        let back: ParamValue = serde_json::from_str(&json).expect("the value reparses");
        assert_eq!(back, value);
    }
    assert_eq!(
        serde_json::to_string(&ParamValue::Null(ParamType::String)).expect("serializes"),
        r#"{"null":"string"}"#
    );
    assert_eq!(
        serde_json::to_string(&ParamValue::Integer(5)).expect("serializes"),
        r#"{"integer":5}"#
    );
    assert_eq!(
        serde_json::to_string(&ParamValue::String("x".to_owned())).expect("serializes"),
        r#"{"string":"x"}"#
    );
}

#[test]
fn param_value_debug_prints_only_the_variant() {
    let values = [
        (ParamValue::Null(ParamType::String), "Null(_)"),
        (ParamValue::String("secret-value".to_owned()), "String(_)"),
        (ParamValue::Integer(-5), "Integer(_)"),
        (ParamValue::Boolean(true), "Boolean(_)"),
        (ParamValue::Decimal("1.5".to_owned()), "Decimal(_)"),
        (ParamValue::Date("2024-02-29".to_owned()), "Date(_)"),
        (
            ParamValue::Timestamp("2024-02-29T00:00:00Z".to_owned()),
            "Timestamp(_)",
        ),
    ];
    for (value, expected) in values {
        let rendered = format!("{value:?}");
        assert_eq!(rendered, expected, "the variant prints, never the value");
    }
    let request = QueryRequest::with_params(
        "select :region",
        10,
        vec![BoundParam {
            name: "region".to_owned(),
            value: ParamValue::String("secret-value".to_owned()),
        }],
    );
    let rendered = format!("{request:?}");
    assert!(
        !rendered.contains("secret-value"),
        "bound values must not render through the request: {rendered}"
    );
}
