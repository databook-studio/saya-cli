use saya_types::ConnectionError;

pub(crate) fn auth() -> ConnectionError {
    ConnectionError::AuthenticationFailed("Snowflake authentication failed".into())
}

pub(crate) fn connect() -> ConnectionError {
    ConnectionError::ConnectionFailed("Snowflake connection failed".into())
}

pub(crate) fn query() -> ConnectionError {
    ConnectionError::QueryFailed("Snowflake query failed".into())
}

pub(crate) fn schema() -> ConnectionError {
    ConnectionError::SchemaFailed("Snowflake schema discovery failed".into())
}

pub(crate) fn interactive() -> ConnectionError {
    ConnectionError::Unsupported(
        "Snowflake external-browser authentication requires interactive mode".into(),
    )
}
