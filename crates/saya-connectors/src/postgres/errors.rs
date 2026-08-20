use saya_types::ConnectionError;
use sqlx::Error;

pub(crate) fn connection(error: Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::authentication_failed("PostgreSQL authentication failed")
    } else {
        ConnectionError::connection_failed("PostgreSQL connection failed")
    }
}

pub(crate) fn query(error: Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::authentication_failed("PostgreSQL authentication failed")
    } else {
        ConnectionError::query_failed("PostgreSQL query failed")
    }
}

fn authentication(error: &Error) -> bool {
    matches!(error, Error::Database(db) if matches!(db.code().as_deref(), Some("28P01" | "28000")))
}

pub(crate) fn schema(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("PostgreSQL schema discovery failed")
}

pub(crate) fn row(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("PostgreSQL schema result was invalid")
}
