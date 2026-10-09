//! The native `duckdb::types::Value` encodings for each validated parameter
//! type, and the decimal-precision refusal that echoes no value.

use chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
use duckdb::types::{Decimal as DuckDecimal, TimeUnit, Value};
use saya_types::{ConnectionError, ParamType, ParamValue};

use crate::duckdb::bind::native_values;

fn epoch_days(date: NaiveDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    date.signed_duration_since(epoch).num_days() as i32
}

#[test]
fn native_values_encode_each_parameter_type() {
    let parsed = crate::binds::parse_bind_values(&[
        ParamValue::Null(ParamType::String),
        ParamValue::String("paris".to_owned()),
        ParamValue::Integer(-5),
        ParamValue::Boolean(true),
        ParamValue::Decimal("19.99".to_owned()),
        ParamValue::Date("2024-02-29".to_owned()),
        ParamValue::Timestamp("2024-01-01T02:00:00+02:00".to_owned()),
    ])
    .unwrap();
    let values = native_values(&parsed).unwrap();
    assert_eq!(values[0], Value::Null);
    assert_eq!(values[1], Value::Text("paris".to_owned()));
    assert_eq!(values[2], Value::BigInt(-5));
    assert_eq!(values[3], Value::Boolean(true));
    // DuckDB DECIMAL(width, scale) carrying the scaled payload: 19.99 → (4, 2, 1999).
    // Text binding was probed and rejected: DuckDB's VARCHAR→DECIMAL cast rounds
    // to the compared column's scale, so `19.99 > 19.988` would compare as false.
    assert_eq!(
        values[4],
        Value::Decimal(DuckDecimal::new(4, 2, 1999).unwrap())
    );
    // Date32 counts days since the Unix epoch.
    assert_eq!(
        values[5],
        Value::Date32(epoch_days(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()))
    );
    // TIMESTAMP is microseconds since epoch at the UTC instant the offset
    // points to: 02:00+02:00 is 00:00Z.
    let micros = FixedOffset::east_opt(2 * 3600)
        .unwrap()
        .with_ymd_and_hms(2024, 1, 1, 2, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
        .timestamp_micros();
    assert_eq!(values[6], Value::Timestamp(TimeUnit::Microsecond, micros));
}

#[test]
fn decimal_beyond_duckdb_precision_refuses_without_echoing_the_value() {
    for text in [
        "1e50".to_owned(),
        "1234567890123456789012345678901234567890123456789".to_owned(),
        "1e-50".to_owned(),
    ] {
        let parsed = crate::binds::parse_bind_values(&[ParamValue::Decimal(text.clone())]).unwrap();
        let error = native_values(&parsed).expect_err("an unrepresentable decimal must refuse");
        let message = error.to_string();
        assert!(
            matches!(error, ConnectionError::QueryFailed(_)),
            "{error:?}"
        );
        assert!(
            !message.contains(&text),
            "the refused value leaked into the error: {message}"
        );
    }
}
