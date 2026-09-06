use saya_types::ConnectionError;

/// The DuckDB message classes that describe a fault in the *submitted SQL* — a
/// name that does not resolve, a binder mismatch, a parse error — rather than a
/// fault in stored data. Their text names identifiers the caller itself
/// supplied, so forwarding it discloses nothing the caller did not already hold.
///
/// Every other class is redacted. `Conversion Error:`, `Out of Range Error:`
/// and `Invalid Input Error:` are excluded on purpose: their text echoes a row
/// value the query tried to coerce, and a row value is exactly what this
/// connector must never forward.
const EXPLAINABLE: [&str; 3] = ["Catalog Error:", "Binder Error:", "Parser Error:"];

/// The longest detail carried back to the caller. DuckDB echoes the offending
/// statement fragment in some messages, so the text is bounded like any other
/// untrusted input rather than trusted to stay short.
const MAX_DETAIL: usize = 300;

/// Maps a query-time DuckDB error. A failure whose message opens with an
/// allow-listed class is forwarded, bounded; everything else is replaced with
/// the fixed "DuckDB query failed" string so an unanticipated class cannot
/// carry a stored value back to the caller.
pub(crate) fn query(error: duckdb::Error) -> ConnectionError {
    match explanation(&error) {
        Some(detail) => ConnectionError::query_failed(format!("DuckDB query failed: {detail}")),
        None => ConnectionError::query_failed("DuckDB query failed"),
    }
}

/// A connection-time failure is never about the SQL, so no driver detail is
/// forwarded.
pub(crate) fn connection(_: duckdb::Error) -> ConnectionError {
    ConnectionError::connection_failed("DuckDB connection failed")
}

/// A fault surfaced while reading a row or a cell can carry a stored value, so
/// it is never forwarded even when the message itself looks safe.
pub(crate) fn decode(_: duckdb::Error) -> ConnectionError {
    ConnectionError::query_failed("DuckDB query failed")
}

fn explanation(error: &duckdb::Error) -> Option<String> {
    let duckdb::Error::DuckDBFailure(_, Some(message)) = error else {
        return None;
    };
    if !EXPLAINABLE
        .into_iter()
        .any(|class| message.starts_with(class))
    {
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

    fn failure(message: &str) -> duckdb::Error {
        duckdb::Error::DuckDBFailure(
            duckdb::ffi::Error::new(duckdb::ffi::DuckDBError),
            Some(message.into()),
        )
    }

    #[test]
    fn catalog_error_names_the_object_and_the_suggestion() {
        let error = failure(
            "Catalog Error: Table with name \"ordrs\" does not exist! Did you mean \"orders\"?",
        );
        let text = query(error).to_string();
        assert!(text.contains("ordrs"), "missing table name: {text}");
        assert!(text.contains("orders"), "did-you-mean dropped: {text}");
    }

    #[test]
    fn unrecognised_class_stays_redacted() {
        let error = failure("Constraint Error: something this connector never considered");
        let text = query(error).to_string();
        assert!(
            !text.contains("never considered"),
            "unvetted message leaked: {text}"
        );
        assert_eq!(text, "query failed: DuckDB query failed");
    }

    #[test]
    fn conversion_error_does_not_leak_row_value() {
        let error =
            failure("Conversion Error: Could not convert string '4111-1111-1111-1111' to INT32");
        let text = query(error).to_string();
        assert!(!text.contains("4111"), "row value leaked: {text}");
        assert_eq!(text, "query failed: DuckDB query failed");
    }

    #[test]
    fn out_of_range_and_invalid_input_are_redacted() {
        for message in [
            "Out of Range Error: value 99999999999 is out of range",
            "Invalid Input Error: value 'secret-row' is invalid",
        ] {
            let text = query(failure(message)).to_string();
            assert_eq!(text, "query failed: DuckDB query failed", "opaque: {text}");
            assert!(!text.contains("secret-row"), "leaked: {text}");
            assert!(!text.contains("99999999999"), "leaked: {text}");
        }
    }

    #[test]
    fn non_failure_variant_is_redacted() {
        let text = query(duckdb::Error::InvalidColumnIndex(7)).to_string();
        assert_eq!(text, "query failed: DuckDB query failed");
    }

    #[test]
    fn detail_is_bounded_on_a_character_boundary() {
        let long = format!("Binder Error: {}", "é".repeat(MAX_DETAIL * 2));
        let text = query(failure(&long)).to_string();
        assert!(text.chars().count() < MAX_DETAIL + 60, "unbounded: {text}");
    }
}
