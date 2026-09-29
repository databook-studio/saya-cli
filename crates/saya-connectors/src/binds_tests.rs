//! Tests for the shared `ParamValue` → engine-native bind mapping: the
//! concrete types each sqlx engine encodes and the text engines must keep
//! verbatim.

use bigdecimal::BigDecimal;
use chrono::{FixedOffset, NaiveDate, TimeZone};
use saya_types::{ConnectionError, ParamValue};

use crate::binds::{BindValue, parse_bind_values};

#[test]
fn primitive_values_map_without_loss() {
    let values = parse_bind_values(&[
        ParamValue::Null,
        ParamValue::String("paris".to_owned()),
        ParamValue::Integer(-5),
        ParamValue::Boolean(true),
    ])
    .unwrap();
    assert!(matches!(values[0], BindValue::Null));
    assert!(matches!(&values[1], BindValue::Str(text) if text == "paris"));
    assert!(matches!(values[2], BindValue::Int(-5)));
    assert!(matches!(values[3], BindValue::Bool(true)));
}

#[test]
fn decimal_keeps_the_parsed_value_and_the_exact_text() {
    // "007" parses to 7 but the text an engine like SQLite must receive stays "007".
    let values = parse_bind_values(&[ParamValue::Decimal("007".to_owned())]).unwrap();
    match &values[0] {
        BindValue::Decimal { value, text } => {
            assert_eq!(*value, BigDecimal::from(7));
            assert_eq!(text, "007");
        }
        other => panic!("expected a decimal bind, got {other:?}"),
    }
}

#[test]
fn date_parses_to_a_chrono_date() {
    let values = parse_bind_values(&[ParamValue::Date("2024-02-29".to_owned())]).unwrap();
    match &values[0] {
        BindValue::Date(date) => {
            assert_eq!(*date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());
        }
        other => panic!("expected a date bind, got {other:?}"),
    }
}

#[test]
fn timestamp_keeps_the_instant_and_the_exact_text() {
    // Lowercase `t`/`z` are valid RFC 3339: the instant parses, and the text
    // engines must receive the validated string untouched.
    let values =
        parse_bind_values(&[ParamValue::Timestamp("2024-02-29t03:04:05z".to_owned())]).unwrap();
    match &values[0] {
        BindValue::Timestamp { value, text } => {
            assert_eq!(
                *value,
                FixedOffset::east_opt(0)
                    .unwrap()
                    .with_ymd_and_hms(2024, 2, 29, 3, 4, 5)
                    .unwrap()
            );
            assert_eq!(text, "2024-02-29t03:04:05z");
        }
        other => panic!("expected a timestamp bind, got {other:?}"),
    }

    let values = parse_bind_values(&[ParamValue::Timestamp(
        "2024-01-01T02:00:00+02:00".to_owned(),
    )])
    .unwrap();
    match &values[0] {
        BindValue::Timestamp { value, text } => {
            assert_eq!(
                *value,
                FixedOffset::east_opt(2 * 3600)
                    .unwrap()
                    .with_ymd_and_hms(2024, 1, 1, 2, 0, 0)
                    .unwrap()
            );
            assert_eq!(text, "2024-01-01T02:00:00+02:00");
        }
        other => panic!("expected a timestamp bind, got {other:?}"),
    }
}

#[test]
fn invalid_text_refuses_without_echoing_the_value() {
    for value in [
        ParamValue::Decimal("not-a-number".to_owned()),
        ParamValue::Date("2024-02-30".to_owned()),
        ParamValue::Timestamp("nope".to_owned()),
    ] {
        let error =
            parse_bind_values(std::slice::from_ref(&value)).expect_err("invalid text must refuse");
        let text = error.to_string();
        assert!(
            matches!(error, ConnectionError::QueryFailed(_)),
            "{error:?}"
        );
        assert!(
            !text.contains("not-a-number") && !text.contains("nope"),
            "an invalid value leaked into the error: {text}"
        );
    }
    // The bind layer re-checks with `BigDecimal::from_str`, which is looser
    // than the CLI grammar: it accepts exponent notation. The strict grammar
    // lives in `ParamValue::parse`; here the value still binds as a number
    // through the protocol, so the looseness is harmless.
    let values = parse_bind_values(&[ParamValue::Decimal("1e5".to_owned())]).unwrap();
    match &values[0] {
        BindValue::Decimal { value, .. } => assert_eq!(*value, BigDecimal::from(100_000)),
        other => panic!("expected a decimal bind, got {other:?}"),
    }
}
