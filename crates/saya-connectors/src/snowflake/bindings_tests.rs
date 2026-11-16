//! Tests for the Snowflake SQL API v2 bindings encoding: the JSON shape per
//! validated type, the epoch conversions (a leap-day date, a timestamp with an
//! offset), the untyped null, and refusals that never echo a value.

use chrono::{FixedOffset, NaiveDate};
use saya_types::ConnectionError;

use crate::binds::{BindValue, parse_bind_values};
use saya_types::{ParamType, ParamValue};

fn decimal(text: &str) -> BindValue {
    match parse_bind_values(&[ParamValue::Decimal(text.to_owned())])
        .unwrap()
        .pop()
        .unwrap()
    {
        BindValue::Decimal { value, text } => BindValue::Decimal { value, text },
        other => panic!("expected a decimal bind, got {other:?}"),
    }
}

fn date(text: &str) -> BindValue {
    match parse_bind_values(&[ParamValue::Date(text.to_owned())])
        .unwrap()
        .pop()
        .unwrap()
    {
        BindValue::Date(date) => BindValue::Date(date),
        other => panic!("expected a date bind, got {other:?}"),
    }
}

fn timestamp(text: &str) -> BindValue {
    match parse_bind_values(&[ParamValue::Timestamp(text.to_owned())])
        .unwrap()
        .pop()
        .unwrap()
    {
        BindValue::Timestamp { value, text } => BindValue::Timestamp { value, text },
        other => panic!("expected a timestamp bind, got {other:?}"),
    }
}

#[test]
fn empty_values_bind_to_an_empty_object() {
    let empty = super::bindings(&[]).unwrap();
    assert_eq!(empty, serde_json::json!({}));
}

#[test]
fn primitives_bind_as_the_documented_types() {
    let bound = super::bindings(&[
        BindValue::Null(ParamType::String),
        BindValue::Str("paris".to_owned()),
        BindValue::Int(12),
        BindValue::Int(-5),
        BindValue::Bool(true),
        BindValue::Bool(false),
    ])
    .unwrap();
    // Keys are 1-based marker positions.
    assert_eq!(
        bound["1"],
        serde_json::json!({"type": "ANY", "value": serde_json::Value::Null})
    );
    assert_eq!(
        bound["2"],
        serde_json::json!({"type": "TEXT", "value": "paris"})
    );
    assert_eq!(
        bound["3"],
        serde_json::json!({"type": "FIXED", "value": "12"})
    );
    assert_eq!(
        bound["4"],
        serde_json::json!({"type": "FIXED", "value": "-5"})
    );
    assert_eq!(
        bound["5"],
        serde_json::json!({"type": "BOOLEAN", "value": "true"})
    );
    assert_eq!(
        bound["6"],
        serde_json::json!({"type": "BOOLEAN", "value": "false"})
    );
}

#[test]
fn decimal_binds_as_fixed_with_the_value_as_given() {
    let bound = super::bindings(&[decimal("007"), decimal("123.45")]).unwrap();
    assert_eq!(
        bound["1"],
        serde_json::json!({"type": "FIXED", "value": "007"})
    );
    assert_eq!(
        bound["2"],
        serde_json::json!({"type": "FIXED", "value": "123.45"})
    );
}

#[test]
fn date_binds_as_epoch_milliseconds_including_a_leap_day() {
    let bound = super::bindings(&[date("1970-01-01"), date("2024-02-29")]).unwrap();
    assert_eq!(
        bound["1"],
        serde_json::json!({"type": "DATE", "value": "0"})
    );
    // 2024-02-29T00:00:00Z is 1709164800 seconds after the epoch.
    assert_eq!(
        bound["2"],
        serde_json::json!({"type": "DATE", "value": "1709164800000"})
    );
}

#[test]
fn timestamp_binds_as_epoch_nanoseconds_plus_the_offset_shift() {
    let bound = super::bindings(&[timestamp("2024-01-01T02:00:00+02:00")]).unwrap();
    // The instant is 2024-01-01T00:00:00Z; the documented TIMESTAMP_TZ format
    // appends the offset in minutes east of UTC shifted by 1440, so +02:00
    // encodes as 1560 and UTC itself as 1440.
    assert_eq!(
        bound["1"],
        serde_json::json!({"type": "TIMESTAMP_TZ", "value": "1704067200000000000 1560"})
    );
    let bound = super::bindings(&[timestamp("2024-01-01T00:00:00Z")]).unwrap();
    assert_eq!(
        bound["1"],
        serde_json::json!({"type": "TIMESTAMP_TZ", "value": "1704067200000000000 1440"})
    );
    // A sub-second timestamp keeps its nanoseconds.
    let bound = super::bindings(&[timestamp("2024-01-01T00:00:00.000000001Z")]).unwrap();
    assert_eq!(
        bound["1"],
        serde_json::json!({"type": "TIMESTAMP_TZ", "value": "1704067200000000001 1440"})
    );
    // A west-of-UTC offset subtracts from 1440, matching the reference driver.
    let bound = super::bindings(&[timestamp("2024-01-01T00:00:00-08:00")]).unwrap();
    assert_eq!(
        bound["1"],
        serde_json::json!({"type": "TIMESTAMP_TZ", "value": "1704096000000000000 0960"})
    );
}

#[test]
fn a_timestamp_outside_the_nanos_range_refuses_without_echoing_the_value() {
    let error = super::bindings(&[timestamp("3000-01-01T00:00:00Z")]).unwrap_err();
    assert!(
        matches!(error, ConnectionError::QueryFailed(_)),
        "{error:?}"
    );
    assert!(
        !error.to_string().contains("3000"),
        "the value leaked into the error: {error}"
    );
}

#[test]
fn bindings_keep_the_parsed_offset_instant() {
    // The bound instant behind the encoded string must be the one the user
    // validated — re-parse one output to prove the epoch math.
    let bound = super::bindings(&[timestamp("2024-02-29t23:59:59+13:00")]).unwrap();
    let encoded = bound["1"]["value"].as_str().unwrap();
    let (nanos, offset) = encoded.split_once(' ').unwrap();
    let instant = chrono::DateTime::parse_from_rfc3339("2024-02-29t23:59:59+13:00").unwrap();
    assert_eq!(
        nanos.parse::<i64>().unwrap(),
        instant
            .with_timezone(&chrono::Utc)
            .timestamp_nanos_opt()
            .unwrap()
    );
    assert_eq!(offset, "2220"); // +13:00 → 780 minutes + 1440
    assert_eq!(instant.offset(), &FixedOffset::east_opt(13 * 3600).unwrap());
    assert_eq!(
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        instant.date_naive()
    );
}
