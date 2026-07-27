use saya_types::ConnectionError;
use sqlx::Error;

pub(crate) fn connection(error: Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::AuthenticationFailed("PostgreSQL authentication failed".into())
    } else {
        ConnectionError::ConnectionFailed("PostgreSQL connection failed".into())
    }
}

pub(crate) fn query(error: Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::AuthenticationFailed("PostgreSQL authentication failed".into())
    } else {
        ConnectionError::QueryFailed("PostgreSQL query failed".into())
    }
}

fn authentication(error: &Error) -> bool {
    matches!(error, Error::Database(db) if matches!(db.code().as_deref(), Some("28P01" | "28000")))
}

pub(crate) fn schema(_: Error) -> ConnectionError {
    ConnectionError::SchemaFailed("PostgreSQL schema discovery failed".into())
}

pub(crate) fn row(_: Error) -> ConnectionError {
    ConnectionError::SchemaFailed("PostgreSQL schema result was invalid".into())
}
