//! Tests for the typed runtime parameter contracts.

use crate::params::{
    BoundParam, MAX_PARAM_DESCRIPTION_BYTES, MAX_PARAMETERS, ParamError, ParamType, ParamValue,
    ParameterSpec, is_valid_param_name,
};
use crate::query::QueryRequest;

use super::parse;

fn spec_named(name: &str) -> ParameterSpec {
    ParameterSpec {
        name: name.to_owned(),
        param_type: ParamType::String,
        required: false,
        description: None,
    }
}

#[test]
fn parse_string_takes_the_raw_text_verbatim() {
    assert_eq!(
        ParamValue::parse(ParamType::String, "hello world"),
        Ok(ParamValue::String("hello world".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::String, ""),
        Ok(ParamValue::String(String::new()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::String, "null"),
        Ok(ParamValue::Null(ParamType::String))
    );
}

#[test]
fn parse_integer_is_strict() {
    assert_eq!(
        ParamValue::parse(ParamType::Integer, "42"),
        Ok(ParamValue::Integer(42))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Integer, "-7"),
        Ok(ParamValue::Integer(-7))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Integer, "null"),
        Ok(ParamValue::Null(ParamType::Integer))
    );
    for bad in [
        "1.0",
        " 5",
        "5 ",
        "+5",
        "--5",
        "",
        "nine",
        "9223372036854775808",
        "-9223372036854775809",
    ] {
        assert!(
            matches!(
                ParamValue::parse(ParamType::Integer, bad),
                Err(ParamError::NotAnInteger(_))
            ),
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn parse_boolean_accepts_only_true_and_false() {
    assert_eq!(
        ParamValue::parse(ParamType::Boolean, "true"),
        Ok(ParamValue::Boolean(true))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Boolean, "false"),
        Ok(ParamValue::Boolean(false))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Boolean, "null"),
        Ok(ParamValue::Null(ParamType::Boolean))
    );
    for bad in ["True", "TRUE", "1", "0", "yes", "", " true"] {
        assert!(
            matches!(
                ParamValue::parse(ParamType::Boolean, bad),
                Err(ParamError::NotABoolean(_))
            ),
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn parse_decimal_shape_and_digit_cap() {
    assert_eq!(
        ParamValue::parse(ParamType::Decimal, "1.5"),
        Ok(ParamValue::Decimal("1.5".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Decimal, "-0.25"),
        Ok(ParamValue::Decimal("-0.25".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Decimal, "42"),
        Ok(ParamValue::Decimal("42".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Decimal, "null"),
        Ok(ParamValue::Null(ParamType::Decimal))
    );
    for bad in [".5", "5.", "-.5", "1.2.3", "1e5", "NaN", "", "-"] {
        assert!(
            matches!(
                ParamValue::parse(ParamType::Decimal, bad),
                Err(ParamError::NotADecimal(_))
            ),
            "{bad:?} must be refused"
        );
    }
    let ok = ParamValue::parse(ParamType::Decimal, &"9".repeat(38));
    assert!(matches!(ok, Ok(ParamValue::Decimal(_))), "38 digits fit");
    let over = ParamValue::parse(ParamType::Decimal, &"9".repeat(39));
    assert!(
        matches!(over, Err(ParamError::NotADecimal(_))),
        "39 digits must be refused"
    );
    let split = ParamValue::parse(
        ParamType::Decimal,
        &format!("{}.{}", "9".repeat(19), "9".repeat(19)),
    );
    assert!(matches!(split, Ok(ParamValue::Decimal(_))), "38 digits fit");
    let split_over = ParamValue::parse(
        ParamType::Decimal,
        &format!("{}.{}", "9".repeat(19), "9".repeat(20)),
    );
    assert!(matches!(split_over, Err(ParamError::NotADecimal(_))));
}

#[test]
fn parse_date_validates_the_calendar() {
    assert_eq!(
        ParamValue::parse(ParamType::Date, "2024-02-29"),
        Ok(ParamValue::Date("2024-02-29".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Date, "null"),
        Ok(ParamValue::Null(ParamType::Date))
    );
    for bad in [
        "2023-02-29", // not a leap year
        "1900-02-29", // century, not divisible by 400
        "2100-02-29", // same rule, future
        "2024-13-01",
        "2024-00-10",
        "2024-04-31",
        "2024-00-00",
        "2024-1-1",
        "20240101",
        "2024-02-2 9",
        "2024/02/29",
        "2024-02-29T00:00:00Z",
        "",
        "0000-00-00",
    ] {
        assert!(
            matches!(
                ParamValue::parse(ParamType::Date, bad),
                Err(ParamError::NotADate(_))
            ),
            "{bad:?} must be refused"
        );
    }
    assert_eq!(
        ParamValue::parse(ParamType::Date, "2000-02-29"),
        Ok(ParamValue::Date("2000-02-29".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Date, "0000-01-01"),
        Ok(ParamValue::Date("0000-01-01".to_owned()))
    );
}

#[test]
fn parse_timestamp_requires_offset_and_valid_parts() {
    assert_eq!(
        ParamValue::parse(ParamType::Timestamp, "2024-02-29T23:59:59Z"),
        Ok(ParamValue::Timestamp("2024-02-29T23:59:59Z".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Timestamp, "2024-02-29T00:00:00+05:30"),
        Ok(ParamValue::Timestamp(
            "2024-02-29T00:00:00+05:30".to_owned()
        ))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Timestamp, "2024-02-29t12:00:00.5-08:00"),
        Ok(ParamValue::Timestamp(
            "2024-02-29t12:00:00.5-08:00".to_owned()
        ))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Timestamp, "2024-02-29T00:00:00z"),
        Ok(ParamValue::Timestamp("2024-02-29T00:00:00z".to_owned()))
    );
    assert_eq!(
        ParamValue::parse(ParamType::Timestamp, "2024-02-29T00:00:00.123456Z"),
        Ok(ParamValue::Timestamp(
            "2024-02-29T00:00:00.123456Z".to_owned()
        ))
    );
    for bad in [
        "2024-02-29T00:00:00",
        "2024-02-29T00:00:00+05",
        "2024-02-29T00:00:00+0500",
        "2024-02-29T00:00:00+25:00",
        "2024-02-29T00:00:00+05:60",
        "2024-02-29T00:00:00-24:00",
        "2024-02-29T24:00:00Z",
        "2024-02-29T00:60:00Z",
        "2024-02-29T00:00:60Z",
        "2024-02-29T00:00:00.",
        "2024-02-29T00:00:00.+05:00",
        "2024-02-29 00:00:00Z",
        "2024-02-29T00:00:00ZZ",
        "2024-02-29T00:00:00.123",
        "2024-02-29T00:00:00+05:30 ",
        "",
    ] {
        assert!(
            matches!(
                ParamValue::parse(ParamType::Timestamp, bad),
                Err(ParamError::NotATimestamp(_))
            ),
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn parse_null_literal_is_case_sensitive() {
    assert!(
        matches!(
            ParamValue::parse(ParamType::Integer, "NULL"),
            Err(ParamError::NotAnInteger(_))
        ),
        "only the exact lowercase literal is the null"
    );
}

#[test]
fn spec_name_and_description_bounds() {
    assert!(is_valid_param_name("region"));
    assert!(is_valid_param_name("_x"));
    assert!(is_valid_param_name("p0"));
    assert!(is_valid_param_name(&"a".repeat(32)));
    for bad in [
        "",
        "Region",
        "9lives",
        "has space",
        "has-dash",
        "dot.name",
        "é",
        &"a".repeat(33),
        "\u{1}",
    ] {
        assert!(!is_valid_param_name(bad), "{bad:?} must be refused");
    }
    ParameterSpec::new("a", ParamType::String, true, None).expect("a bare spec validates");
    ParameterSpec::new(
        "a",
        ParamType::String,
        true,
        Some("d".repeat(MAX_PARAM_DESCRIPTION_BYTES)),
    )
    .expect("a description at the cap fits");
    let over = ParameterSpec::new(
        "a",
        ParamType::String,
        true,
        Some("d".repeat(MAX_PARAM_DESCRIPTION_BYTES + 1)),
    );
    assert_eq!(over, Err(ParamError::DescriptionTooLong));
    let control = ParameterSpec::new("a", ParamType::String, true, Some("bad\u{1}".to_owned()));
    assert_eq!(control, Err(ParamError::DescriptionControlCharacter));
    let bad_name = ParameterSpec::new("Region", ParamType::String, true, None);
    assert_eq!(bad_name, Err(ParamError::InvalidName("Region".to_owned())));
    // A struct built without `new` is still checked by `validate`.
    let raw = spec_named("Region");
    assert_eq!(
        raw.validate(),
        Err(ParamError::InvalidName("Region".to_owned()))
    );
}

#[test]
fn spec_list_bounds_and_uniqueness() {
    let specs = |count: usize| {
        (0..count)
            .map(|i| spec_named(&format!("p{i:02}")))
            .collect::<Vec<_>>()
    };
    assert_eq!(ParameterSpec::validate_list(&[]), Ok(()));
    ParameterSpec::validate_list(&specs(MAX_PARAMETERS)).expect("32 distinct specs fit");
    assert_eq!(
        ParameterSpec::validate_list(&specs(MAX_PARAMETERS + 1)),
        Err(ParamError::TooManySpecs(MAX_PARAMETERS + 1))
    );
    let duplicates = vec![spec_named("region"), spec_named("region")];
    assert_eq!(
        ParameterSpec::validate_list(&duplicates),
        Err(ParamError::DuplicateName("region".to_owned()))
    );
    let misnamed = vec![spec_named("region"), spec_named("Region")];
    assert!(matches!(
        ParameterSpec::validate_list(&misnamed),
        Err(ParamError::InvalidName(_))
    ));
}

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

#[test]
fn parse_module_rejects_untrimmed_input() {
    for (param_type, raw) in [
        (ParamType::Integer, " 5"),
        (ParamType::Decimal, "1.5 "),
        (ParamType::Date, " 2024-02-29"),
        (ParamType::Timestamp, "2024-02-29T00:00:00Z "),
    ] {
        assert!(
            ParamValue::parse(param_type, raw).is_err(),
            "{raw:?} must not be trimmed into acceptance"
        );
    }
    assert!(parse::integer("007").is_ok(), "leading zeros stay digits");
}
