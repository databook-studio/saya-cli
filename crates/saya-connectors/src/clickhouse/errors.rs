use reqwest::{Error, StatusCode};
use saya_types::ConnectionError;

pub(crate) fn build() -> ConnectionError {
    ConnectionError::connection_failed("ClickHouse connection could not be initialized")
}

pub(crate) fn query_timeout() -> ConnectionError {
    ConnectionError::query_failed("ClickHouse query timed out")
}

/// Transport failure while opening a connection: always a connection-level
/// outcome, never a query one, so the health check reads as a connection error.
pub(crate) fn transport_connect(error: Error) -> ConnectionError {
    if error.is_timeout() {
        ConnectionError::connection_failed("ClickHouse connection timed out")
    } else {
        ConnectionError::connection_failed("ClickHouse connection failed")
    }
}

/// Transport failure while running a query. A timeout is a query outcome; a
/// connect failure is still surfaced as a connection problem; anything else is
/// a query failure. The HTTP body is never read into the message, so a server
/// exception cannot carry credentials or the query text back to the caller.
pub(crate) fn transport_query(error: Error) -> ConnectionError {
    if error.is_timeout() {
        ConnectionError::query_failed("ClickHouse query timed out")
    } else if error.is_connect() {
        ConnectionError::connection_failed("ClickHouse connection failed")
    } else {
        ConnectionError::query_failed("ClickHouse query failed")
    }
}

/// Failure decoding the response body of a query: a timeout is the client
/// bound, a decode failure is an unreadable result, and anything else is a
/// generic query failure. No body content reaches the message.
pub(crate) fn body(error: Error) -> ConnectionError {
    if error.is_timeout() {
        ConnectionError::query_failed("ClickHouse query timed out")
    } else if error.is_decode() {
        ConnectionError::query_failed("ClickHouse returned an unreadable result")
    } else {
        ConnectionError::query_failed("ClickHouse query failed")
    }
}

pub(crate) fn connect_status(status: StatusCode) -> ConnectionError {
    match status.as_u16() {
        401 | 403 => ConnectionError::authentication_failed("ClickHouse authentication failed"),
        _ => ConnectionError::connection_failed("ClickHouse connection failed"),
    }
}

pub(crate) fn query_status(status: StatusCode) -> ConnectionError {
    match status.as_u16() {
        401 | 403 => ConnectionError::authentication_failed("ClickHouse authentication failed"),
        s if s >= 500 => ConnectionError::connection_failed("ClickHouse server error"),
        _ => ConnectionError::query_failed("ClickHouse query failed"),
    }
}

pub(crate) fn schema() -> ConnectionError {
    ConnectionError::schema_failed("ClickHouse schema discovery failed")
}

pub(crate) fn schema_timeout() -> ConnectionError {
    ConnectionError::schema_failed("ClickHouse schema discovery timed out")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_statuses_map_to_authentication_failed() {
        for code in [401, 403] {
            let status = StatusCode::from_u16(code).unwrap();
            assert!(matches!(
                connect_status(status),
                ConnectionError::AuthenticationFailed(_)
            ));
            assert!(matches!(
                query_status(status),
                ConnectionError::AuthenticationFailed(_)
            ));
        }
    }

    #[test]
    fn client_statuses_map_to_query_failed_and_server_statuses_to_connection_failed() {
        // A 4xx is a SQL fault the caller may be able to repair; a 5xx is a
        // server fault that is not about the SQL, so it stops reading as a
        // query failure and surfaces as a connection-level problem instead.
        let server = StatusCode::from_u16(500).unwrap();
        assert!(matches!(
            connect_status(server),
            ConnectionError::ConnectionFailed(_)
        ));
        assert!(matches!(
            query_status(server),
            ConnectionError::ConnectionFailed(_)
        ));
        let query_text = query_status(server).to_string();
        assert!(query_text.contains("server error"), "opaque: {query_text}");

        let not_found = StatusCode::from_u16(404).unwrap();
        assert!(matches!(
            connect_status(not_found),
            ConnectionError::ConnectionFailed(_)
        ));
        assert!(matches!(
            query_status(not_found),
            ConnectionError::QueryFailed(_)
        ));
    }

    #[test]
    fn messages_never_carry_secrets() {
        // Every mapped message is a fixed string; none interpolates the HTTP
        // body, the URL, the user, or the password, so a planted secret in any
        // of those cannot leak through an error.
        let planted = "PLANTED_SECRET_VALUE";
        let cases = [
            build(),
            schema(),
            schema_timeout(),
            connect_status(StatusCode::from_u16(401).unwrap()),
            query_status(StatusCode::from_u16(500).unwrap()),
        ];
        for error in cases {
            let text = error.to_string();
            assert!(!text.contains(planted), "secret leaked: {text}");
            assert!(!text.contains("clickhouse://"), "url leaked: {text}");
            assert!(!text.contains("password"), "secret name leaked: {text}");
        }
    }
}
