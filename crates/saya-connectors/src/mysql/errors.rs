use saya_types::ConnectionError;

pub(crate) fn connection(error: sqlx::Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::authentication_failed("MySQL authentication failed")
    } else {
        ConnectionError::connection_failed("MySQL connection failed")
    }
}

pub(crate) fn query(error: sqlx::Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::authentication_failed("MySQL authentication failed")
    } else {
        ConnectionError::query_failed("MySQL query failed")
    }
}

pub(crate) fn schema(_: sqlx::Error) -> ConnectionError {
    ConnectionError::schema_failed("MySQL schema discovery failed")
}

fn authentication(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if matches!(database.code().as_deref(), Some("1045" | "28000")))
}
