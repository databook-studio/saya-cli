use saya_types::ConnectionError;

pub(crate) fn auth() -> ConnectionError {
    ConnectionError::authentication_failed("Snowflake authentication failed")
}

pub(crate) fn connect() -> ConnectionError {
    ConnectionError::connection_failed("Snowflake connection failed")
}

pub(crate) fn query() -> ConnectionError {
    ConnectionError::query_failed("Snowflake query failed")
}

pub(crate) fn schema() -> ConnectionError {
    ConnectionError::schema_failed("Snowflake schema discovery failed")
}

pub(crate) fn interactive() -> ConnectionError {
    ConnectionError::unsupported(
        "Snowflake external-browser authentication requires interactive mode",
    )
}
