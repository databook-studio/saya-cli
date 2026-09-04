use reqwest::{Error, StatusCode};
use saya_types::ConnectionError;

pub(crate) fn build() -> ConnectionError {
    ConnectionError::connection_failed("BigQuery connection could not be initialized")
}

pub(crate) fn config() -> ConnectionError {
    ConnectionError::invalid_configuration("BigQuery service-account key is invalid")
}

pub(crate) fn auth() -> ConnectionError {
    ConnectionError::authentication_failed("BigQuery authentication failed")
}

pub(crate) fn query_timeout() -> ConnectionError {
    ConnectionError::query_failed("BigQuery query timed out")
}

/// Transport failure while running a query. A timeout is a query outcome; a
/// connect failure is still a connection problem; anything else is a query
/// failure. No body content reaches the message.
pub(crate) fn transport_query(error: Error) -> ConnectionError {
    if error.is_timeout() {
        ConnectionError::query_failed("BigQuery query timed out")
    } else if error.is_connect() {
        ConnectionError::connection_failed("BigQuery connection failed")
    } else {
        ConnectionError::query_failed("BigQuery query failed")
    }
}

pub(crate) fn body(error: Error) -> ConnectionError {
    if error.is_timeout() {
        ConnectionError::query_failed("BigQuery query timed out")
    } else if error.is_decode() {
        ConnectionError::query_failed("BigQuery returned an unreadable result")
    } else {
        ConnectionError::query_failed("BigQuery query failed")
    }
}

pub(crate) fn query_status(status: StatusCode) -> ConnectionError {
    match status.as_u16() {
        401 | 403 => ConnectionError::authentication_failed("BigQuery authentication failed"),
        _ => ConnectionError::query_failed("BigQuery query failed"),
    }
}

pub(crate) fn schema() -> ConnectionError {
    ConnectionError::schema_failed("BigQuery schema discovery failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_statuses_map_to_authentication_failed() {
        for code in [401, 403] {
            let status = StatusCode::from_u16(code).unwrap();
            assert!(matches!(
                query_status(status),
                ConnectionError::AuthenticationFailed(_)
            ));
        }
    }

    #[test]
    fn other_statuses_map_to_query_failed() {
        let server = StatusCode::from_u16(500).unwrap();
        assert!(matches!(
            query_status(server),
            ConnectionError::QueryFailed(_)
        ));
    }

    #[test]
    fn messages_never_carry_secrets() {
        // Every mapped message is a fixed string; none interpolates the HTTP
        // body, the URL, the access token, or the service-account key, so a
        // planted secret in any of those cannot leak through an error.
        let planted = "PLANTED_SECRET_VALUE";
        let cases = [
            build(),
            config(),
            auth(),
            schema(),
            query_status(StatusCode::from_u16(401).unwrap()),
            query_status(StatusCode::from_u16(500).unwrap()),
        ];
        for error in cases {
            let text = error.to_string();
            assert!(!text.contains(planted), "secret leaked: {text}");
            assert!(!text.contains("Bearer"), "token kind leaked: {text}");
            assert!(!text.contains("private_key"), "key name leaked: {text}");
        }
    }
}
