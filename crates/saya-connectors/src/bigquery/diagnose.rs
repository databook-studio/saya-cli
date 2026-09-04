use reqwest::StatusCode;
use saya_types::ConnectionError;
use serde_json::Value;

use super::errors;

/// The longest error text carried back to the caller. BigQuery echoes the
/// submitted statement in some messages, so the text is bounded like any other
/// untrusted input rather than being trusted to stay short.
const MAX_DETAIL: usize = 300;

/// Error reasons whose message describes the *submitted statement* — an
/// unrecognised name, a missing table, a syntax fault, a bound the query
/// exceeded — rather than describing stored data.
///
/// The reason code is the allow-list key, not the message: an unanticipated
/// failure carries a reason outside this set and stays redacted, so a message
/// shape that was never considered cannot leak through. For these reasons the
/// message names identifiers the caller itself submitted and bounds it broke,
/// which is the difference between correcting the query and guessing at it.
const EXPLAINABLE: [&str; 9] = [
    "invalidQuery",
    "invalid",
    "notFound",
    "resourcesExceeded",
    "responseTooLarge",
    "billingTierLimitExceeded",
    "bytesBilledLimitExceeded",
    "quotaExceeded",
    "rateLimitExceeded",
];

/// Classifies a failed query response. Authentication statuses keep their fixed
/// messages and never echo the body — a credential fault needs no detail from
/// the server, and that is the one response where echoing would be riskiest.
pub(crate) fn query_failure(status: StatusCode, body: &str) -> ConnectionError {
    match status.as_u16() {
        401 | 403 => errors::query_status(status),
        _ => match explanation(body) {
            Some(detail) => {
                ConnectionError::query_failed(format!("BigQuery query failed: {detail}"))
            }
            None => ConnectionError::query_failed("BigQuery query failed"),
        },
    }
}

/// Reads the reason and message out of Google's error envelope, returning the
/// message only when the reason is one this connector chooses to explain.
fn explanation(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?;
    let reason = error
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .and_then(|first| first.get("reason"))
        .and_then(Value::as_str)?;
    if !EXPLAINABLE.contains(&reason) {
        return None;
    }
    let message = error.get("message").and_then(Value::as_str)?.trim();
    if message.is_empty() {
        return None;
    }
    Some(bounded(message))
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

    fn envelope(code: u16, reason: &str, message: &str) -> String {
        serde_json::json!({
            "error": {
                "code": code,
                "message": message,
                "errors": [{"message": message, "reason": reason}],
            }
        })
        .to_string()
    }

    fn status(code: u16) -> StatusCode {
        StatusCode::from_u16(code).unwrap()
    }

    #[test]
    fn missing_table_names_the_table() {
        let body = envelope(
            404,
            "notFound",
            "Not found: Table proj:set.absent was not found",
        );
        let text = query_failure(status(404), &body).to_string();
        assert!(text.contains("Table proj:set.absent"), "opaque: {text}");
    }

    #[test]
    fn unrecognised_name_reaches_the_caller() {
        let body = envelope(
            400,
            "invalidQuery",
            "Unrecognized name: nonexistent_col at [1:8]",
        );
        let text = query_failure(status(400), &body).to_string();
        assert!(text.contains("Unrecognized name"), "opaque: {text}");
        assert!(text.contains("nonexistent_col"), "opaque: {text}");
    }

    #[test]
    fn unknown_reason_stays_redacted() {
        // A reason this connector never considered may carry anything, so the
        // message is dropped rather than forwarded on the chance it is safe.
        let body = envelope(
            400,
            "someReasonNobodyAnticipated",
            "row value 4111-1111-1111-1111",
        );
        let text = query_failure(status(400), &body).to_string();
        assert!(!text.contains("4111"), "unvetted message leaked: {text}");
        assert_eq!(text, "query failed: BigQuery query failed");
    }

    #[test]
    fn authentication_statuses_never_echo_the_body() {
        for code in [401, 403] {
            let body = envelope(code, "invalidQuery", "PLANTED_SECRET_VALUE");
            let text = query_failure(status(code), &body).to_string();
            assert!(!text.contains("PLANTED_SECRET_VALUE"), "leaked: {text}");
        }
    }

    #[test]
    fn detail_is_bounded_and_splits_on_a_character_boundary() {
        let long = "é".repeat(MAX_DETAIL * 2);
        let body = envelope(400, "invalidQuery", &long);
        let text = query_failure(status(400), &body).to_string();
        assert!(
            text.chars().count() < MAX_DETAIL + 60,
            "unbounded: {}",
            text.len()
        );
    }

    #[test]
    fn unparseable_body_is_not_a_panic() {
        let text = query_failure(status(500), "<html>gateway error</html>").to_string();
        assert_eq!(text, "query failed: BigQuery query failed");
    }
}
