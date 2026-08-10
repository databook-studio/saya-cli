use saya_types::ConnectionError;
use sqlx::Error;

pub(crate) fn connection(_: Error) -> ConnectionError {
    ConnectionError::ConnectionFailed("SQLite connection failed".into())
}

pub(crate) fn query(_: Error) -> ConnectionError {
    ConnectionError::QueryFailed("SQLite query failed".into())
}

pub(crate) fn schema(_: Error) -> ConnectionError {
    ConnectionError::SchemaFailed("SQLite schema discovery failed".into())
}

pub(crate) fn row(_: Error) -> ConnectionError {
    ConnectionError::SchemaFailed("SQLite schema result was invalid".into())
}
