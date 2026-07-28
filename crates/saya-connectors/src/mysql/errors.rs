use saya_types::ConnectionError;

pub(crate) fn connection(error: sqlx::Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::AuthenticationFailed("MySQL authentication failed".into())
    } else {
        ConnectionError::ConnectionFailed("MySQL connection failed".into())
    }
}

pub(crate) fn query(error: sqlx::Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::AuthenticationFailed("MySQL authentication failed".into())
    } else {
        ConnectionError::QueryFailed("MySQL query failed".into())
    }
}

pub(crate) fn schema(_: sqlx::Error) -> ConnectionError {
    ConnectionError::SchemaFailed("MySQL schema discovery failed".into())
}

fn authentication(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if matches!(database.code().as_deref(), Some("1045" | "28000")))
}
