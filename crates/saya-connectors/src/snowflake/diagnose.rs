use saya_types::ConnectionError;
use serde_json::Value;

use super::errors;

/// The longest error text carried back to the caller. Snowflake echoes the
/// submitted statement in some messages, so the text is bounded like any other
/// untrusted input rather than trusted to stay short.
const MAX_DETAIL: usize = 300;

/// Snowflake SQL error codes whose `message` describes the *submitted
/// statement* — an object that does not exist, an invalid identifier, a syntax
/// fault, an ambiguous column, a type mismatch on a named expression, a missing
/// warehouse — rather than describing stored data.
///
/// The code is the allow-list key, not the message: an unanticipated failure
/// carries a code outside this set and stays redacted, so a message shape that
/// was never considered cannot leak through. For these codes the message names
/// identifiers the caller itself submitted, which is the difference between
/// correcting the query and guessing at it.
const EXPLAINABLE: [&str; 6] = [
    "002003", // object does not exist or not authorized
    "000904", // invalid identifier
    "001003", // syntax error
    "002040", // ambiguous column
    "001044", // expression type mismatch
    "000606", // no active warehouse
];

/// Classifies a failed query response whose body is already parsed. The
/// envelope differs by transport: the v2 SQL API carries `code` and `message`
/// at the top level, while the legacy session API nests them under `data`. Both
/// are read; whichever carries a code wins, and a body without a code stays
/// redacted.
pub(crate) fn query_failure(value: &Value) -> ConnectionError {
    match explanation(value) {
        Some(detail) => ConnectionError::query_failed(format!("Snowflake query failed: {detail}")),
        None => errors::query(),
    }
}

fn explanation(value: &Value) -> Option<String> {
    let (code, message) = pair(value).or_else(|| value.get("data").and_then(pair))?;
    if !EXPLAINABLE.contains(&code.as_str()) {
        return None;
    }
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    Some(bounded(message))
}

/// Reads the `code` and `message` siblings out of one envelope level.
fn pair(value: &Value) -> Option<(String, String)> {
    let code = value.get("code").and_then(Value::as_str)?.to_owned();
    let message = value.get("message").and_then(Value::as_str)?.to_owned();
    Some((code, message))
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
    use serde_json::json;

    fn body(code: &str, message: &str) -> Value {
        json!({"code": code, "message": message, "success": false})
    }

    fn data_body(code: &str, message: &str) -> Value {
        json!({"success": false, "data": {"code": code, "message": message}})
    }

    #[test]
    fn names_the_object_for_missing_object_code() {
        let value = body(
            "002003",
            "SQL compilation error:\nObject 'ORDRS' does not exist or not authorized.",
        );
        let text = query_failure(&value).to_string();
        assert!(text.contains("ORDRS"), "opaque: {text}");
        assert!(text.contains("does not exist"), "opaque: {text}");
    }

    #[test]
    fn legacy_envelope_under_data_is_read_too() {
        let value = data_body(
            "001003",
            "syntax error line 1 at position 0 unexpected 'SELCT'",
        );
        let text = query_failure(&value).to_string();
        assert!(text.contains("syntax error"), "opaque: {text}");
        assert!(text.contains("SELCT"), "opaque: {text}");
    }

    #[test]
    fn unrecognised_code_stays_redacted() {
        let value = body("999999", "row value 4111-1111-1111-1111");
        let text = query_failure(&value).to_string();
        assert!(!text.contains("4111"), "unvetted message leaked: {text}");
        assert_eq!(text, "query failed: Snowflake query failed");
    }

    #[test]
    fn conversion_error_does_not_leak_row_value() {
        let value = body(
            "100038",
            "Numeric value '4111-1111-1111-1111' is not recognized",
        );
        let text = query_failure(&value).to_string();
        assert!(!text.contains("4111"), "row value leaked: {text}");
        assert_eq!(text, "query failed: Snowflake query failed");
    }

    #[test]
    fn no_code_field_stays_redacted() {
        let value = json!({"message": "LEAK-MARKER"});
        let text = query_failure(&value).to_string();
        assert!(!text.contains("LEAK-MARKER"), "leaked: {text}");
        assert_eq!(text, "query failed: Snowflake query failed");
    }

    #[test]
    fn empty_message_falls_back_to_generic() {
        let value = body("002003", "");
        assert_eq!(
            query_failure(&value).to_string(),
            "query failed: Snowflake query failed"
        );
    }

    #[test]
    fn detail_is_bounded_on_a_character_boundary() {
        let long = "é".repeat(MAX_DETAIL * 2);
        let value = body("002003", &long);
        let text = query_failure(&value).to_string();
        assert!(text.chars().count() < MAX_DETAIL + 60, "unbounded: {text}");
    }
}
