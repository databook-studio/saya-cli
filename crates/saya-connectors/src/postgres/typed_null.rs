//! The typed-null bind for PostgreSQL (B1f): a null carries its parameter's
//! declared type, so the server sees the declared parameter type instead of
//! inferring one from context. The declared type decides the concrete
//! `Option<T>` the value encodes as — the same table the unit test pins
//! through [`Encode::produces`], which sqlx consults when the argument's
//! type is declared.

use bigdecimal::BigDecimal;
use chrono::{DateTime, FixedOffset, NaiveDate};
use saya_types::ParamType;
use sqlx::{
    Encode, Postgres, Type,
    encode::IsNull,
    error::BoxDynError,
    postgres::{PgArgumentBuffer, PgTypeInfo},
};

/// One null bound as its declared parameter type. A null encodes no bytes;
/// the declared type travels through [`Encode::produces`], which sqlx
/// consults for the argument's declared type — the same concrete `Option<T>`
/// types an `Option::None` of each declared type encodes as.
pub(crate) struct TypedNull(pub(crate) ParamType);

impl<'q> Encode<'q, Postgres> for TypedNull {
    fn encode_by_ref(&self, _buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        Ok(IsNull::Yes)
    }

    fn produces(&self) -> Option<PgTypeInfo> {
        Some(match self.0 {
            ParamType::String => <Option<String> as Type<Postgres>>::type_info(),
            ParamType::Integer => <Option<i64> as Type<Postgres>>::type_info(),
            ParamType::Boolean => <Option<bool> as Type<Postgres>>::type_info(),
            ParamType::Decimal => <Option<BigDecimal> as Type<Postgres>>::type_info(),
            ParamType::Date => <Option<NaiveDate> as Type<Postgres>>::type_info(),
            ParamType::Timestamp => <Option<DateTime<FixedOffset>> as Type<Postgres>>::type_info(),
            // A declared type this connector does not map yet binds untyped,
            // the behavior a null had before declared types existed.
            _ => <Option<String> as Type<Postgres>>::type_info(),
        })
    }
}

impl Type<Postgres> for TypedNull {
    /// The trait-bound fallback sqlx only reaches through `produces()`,
    /// which is always `Some` here. An unknown future declared type binds
    /// untyped (text), the behavior a null had before the declared type
    /// existed.
    fn type_info() -> PgTypeInfo {
        <Option<String> as Type<Postgres>>::type_info()
    }
}

#[cfg(test)]
#[path = "typed_null_tests.rs"]
mod tests;
