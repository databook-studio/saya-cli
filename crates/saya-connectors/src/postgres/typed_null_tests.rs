//! The typed-null bind shapes (B1f): the declared parameter type decides
//! the PostgreSQL parameter type the null is declared as — the concrete
//! `Option<T>` each declared type encodes as, pinned by its system name.

use super::TypedNull;
use saya_types::ParamType;
use sqlx::{Encode as _, TypeInfo as _};

#[test]
fn typed_nulls_produce_the_postgres_type_of_their_declared_type() {
    let expected = [
        (ParamType::String, "TEXT"),
        (ParamType::Integer, "INT8"),
        (ParamType::Boolean, "BOOL"),
        (ParamType::Decimal, "NUMERIC"),
        (ParamType::Date, "DATE"),
        (ParamType::Timestamp, "TIMESTAMPTZ"),
    ];
    for (param_type, name) in expected {
        let produced = TypedNull(param_type)
            .produces()
            .expect("a typed null always declares its type");
        assert_eq!(produced.name(), name, "{param_type}");
    }
}

#[test]
fn a_typed_null_encodes_no_bytes_like_any_null() {
    use sqlx::Encode as _;
    let mut buffer = sqlx::postgres::PgArgumentBuffer::default();
    let encoded = TypedNull(ParamType::Integer)
        .encode_by_ref(&mut buffer)
        .expect("a null encodes without error");
    assert!(
        matches!(encoded, sqlx::encode::IsNull::Yes),
        "a null writes no bytes"
    );
}
