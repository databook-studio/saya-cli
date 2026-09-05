use reqwest::{StatusCode, header::HeaderMap};
use saya_types::ConnectionError;

use super::errors;

/// The longest error text carried back to the caller. ClickHouse echoes the
/// submitted statement in some messages, so the text is bounded like any other
/// untrusted input rather than trusted to stay short.
const MAX_DETAIL: usize = 300;

/// ClickHouse exception codes whose message describes the *submitted
/// statement* — a table or identifier or function the caller named that does
/// not resolve, a syntax fault, a type mismatch on a named argument, an
/// ambiguous column, or a server-side resource bound (memory, time) the query
/// exceeded — rather than describing stored data.
///
/// The code is the allow-list key, not the message: a failure carrying a code
/// outside this set stays redacted, so a message shape that was never
/// considered cannot leak through. Conversion and parse faults (codes 6, 27,
/// 72) are excluded on purpose: their text echoes the row value the query tried
/// to coerce, and a row value is exactly what this connector must never
/// forward.
const EXPLAINABLE: [u16; 11] = [
    60,  // UNKNOWN_TABLE
    47,  // UNKNOWN_IDENTIFIER
    62,  // SYNTAX_ERROR
    46,  // UNKNOWN_FUNCTION
    81,  // UNKNOWN_DATABASE
    42,  // NUMBER_OF_ARGUMENTS_DOESNT_MATCH
    43,  // ILLEGAL_TYPE_OF_ARGUMENT
    352, // AMBIGUOUS_COLUMN_NAME
    10,  // NOT_FOUND_COLUMN_IN_BLOCK
    241, // MEMORY_LIMIT_EXCEEDED
    159, // TIMEOUT_EXCEEDED
];

/// Classifies a failed query response. Authentication statuses and 5xx server
/// faults are not SQL faults and keep fixed messages; a 4xx SQL fault is
/// classified on the exception code, forwarding the message only for an
/// allow-listed code. The header is preferred over the body for the code
/// because it is cheaper to read and survives a gateway that rewrites the body.
pub(crate) fn query_failure(
    status: StatusCode,
    headers: &HeaderMap,
    body: &str,
) -> ConnectionError {
    let code = status.as_u16();
    if code == 401 || code == 403 || status.is_server_error() {
        return errors::query_status(status);
    }
    match classify(headers, body) {
        Some(detail) => ConnectionError::query_failed(format!("ClickHouse query failed: {detail}")),
        None => errors::query_status(status),
    }
}

fn classify(headers: &HeaderMap, body: &str) -> Option<String> {
    let code = code_from_header(headers).or_else(|| code_from_body(body))?;
    if !EXPLAINABLE.contains(&code) {
        return None;
    }
    let message = message_from_body(body)?.trim();
    if message.is_empty() {
        return None;
    }
    Some(bounded(message))
}

fn code_from_header(headers: &HeaderMap) -> Option<u16> {
    headers
        .get("X-ClickHouse-Exception-Code")?
        .to_str()
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
}

/// The leading `Code: <n>.` in a ClickHouse exception body.
fn code_from_body(body: &str) -> Option<u16> {
    let rest = body.trim_start().strip_prefix("Code: ")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u16>().ok()
}

/// The text after `DB::Exception: ` on its first line. A body without that
/// marker yields `None`, so a gateway page that happens to carry an allow-listed
/// header code is still redacted rather than forwarded.
fn message_from_body(body: &str) -> Option<&str> {
    let after = body.split_once("DB::Exception: ")?.1;
    Some(after.split('\n').next().unwrap_or(after))
}

/// Truncates on a character boundary so a multi-byte message cannot panic.
fn bounded(message: &str) -> String {
    if message.chars().count() <= MAX_DETAIL {
        return message.to_owned();
    }
    let kept: String = message.chars().take(MAX_DETAIL).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderName};

    fn header(code: u16) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-clickhouse-exception-code"),
            code.to_string().parse().unwrap(),
        );
        headers
    }

    fn body(code: u16, message: &str, name: &str) -> String {
        format!("Code: {code}. DB::Exception: {message}. ({name})")
    }

    fn status(code: u16) -> StatusCode {
        StatusCode::from_u16(code).unwrap()
    }

    #[test]
    fn names_the_missing_table() {
        let headers = header(60);
        let body = body(60, "Table default.orders doesn't exist", "UNKNOWN_TABLE");
        let text = query_failure(status(404), &headers, &body).to_string();
        assert!(text.contains("orders"), "opaque: {text}");
        assert!(text.contains("doesn't exist"), "opaque: {text}");
    }

    #[test]
    fn reads_code_from_body_when_header_absent() {
        let headers = HeaderMap::new();
        let body = body(47, "Unknown identifier 'foo'", "UNKNOWN_IDENTIFIER");
        let text = query_failure(status(400), &headers, &body).to_string();
        assert!(text.contains("foo"), "opaque: {text}");
    }

    #[test]
    fn unrecognised_code_stays_redacted() {
        let headers = header(999);
        let body = "Code: 999. DB::Exception: row value 4111-1111-1111-1111";
        let text = query_failure(status(400), &headers, body).to_string();
        assert!(!text.contains("4111"), "unvetted message leaked: {text}");
        assert_eq!(text, "query failed: ClickHouse query failed");
    }

    #[test]
    fn conversion_error_does_not_leak_row_value() {
        let headers = header(6);
        let body = "Code: 6. DB::Exception: Cannot parse string '4111-1111-1111-1111' \
                    as UInt64. (CANNOT_PARSE_QUOTED_STRING)";
        let text = query_failure(status(400), &headers, body).to_string();
        assert!(!text.contains("4111"), "row value leaked: {text}");
        assert_eq!(text, "query failed: ClickHouse query failed");
    }

    #[test]
    fn body_without_db_exception_is_redacted() {
        let headers = header(60);
        let text = query_failure(status(400), &headers, "<html>gateway error</html>").to_string();
        assert_eq!(text, "query failed: ClickHouse query failed");
    }

    #[test]
    fn server_fault_is_not_a_sql_fault() {
        let headers = header(60);
        let body = body(60, "Table x doesn't exist", "UNKNOWN_TABLE");
        let error = query_failure(status(500), &headers, &body);
        assert!(
            matches!(error, ConnectionError::ConnectionFailed(_)),
            "5xx read as a SQL fault: {error}"
        );
        assert!(
            !error.to_string().contains("x doesn't exist"),
            "server fault diagnosed: {error}"
        );
    }

    #[test]
    fn auth_status_keeps_fixed_message_and_never_echoes_body() {
        let headers = header(60);
        let body = body(60, "PLANTED_SECRET_VALUE", "UNKNOWN_TABLE");
        let text = query_failure(status(401), &headers, &body).to_string();
        assert!(
            !text.contains("PLANTED_SECRET_VALUE"),
            "auth echoed body: {text}"
        );
        assert!(text.contains("authentication failed"), "not auth: {text}");
    }

    #[test]
    fn detail_is_bounded_on_a_character_boundary() {
        let headers = header(60);
        let long = format!("Table {} doesn't exist", "é".repeat(MAX_DETAIL * 2));
        let body = body(60, &long, "UNKNOWN_TABLE");
        let text = query_failure(status(400), &headers, &body).to_string();
        assert!(text.chars().count() < MAX_DETAIL + 60, "unbounded: {text}");
    }
}
