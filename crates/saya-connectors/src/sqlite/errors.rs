use saya_types::ConnectionError;
use sqlx::Error;

pub(crate) fn connection(_: Error) -> ConnectionError {
    ConnectionError::connection_failed("SQLite connection failed")
}

pub(crate) fn query(_: Error) -> ConnectionError {
    ConnectionError::query_failed("SQLite query failed")
}

pub(crate) fn schema(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("SQLite schema discovery failed")
}

pub(crate) fn row(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("SQLite schema result was invalid")
}
