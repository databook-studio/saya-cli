use super::*;

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
