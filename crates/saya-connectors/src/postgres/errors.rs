use saya_types::ConnectionError;
use sqlx::Error;

pub(crate) fn connection(error: Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::authentication_failed("PostgreSQL authentication failed")
    } else {
        ConnectionError::connection_failed("PostgreSQL connection failed")
    }
}

pub(crate) fn query(error: Error) -> ConnectionError {
    if authentication(&error) {
        return ConnectionError::authentication_failed("PostgreSQL authentication failed");
    }
    match error {
        Error::Database(db) => match classify(db.code().as_deref(), db.message()) {
            Some(detail) => {
                ConnectionError::query_failed(format!("PostgreSQL query failed: {detail}"))
            }
            None => ConnectionError::query_failed("PostgreSQL query failed"),
        },
        _ => ConnectionError::query_failed("PostgreSQL query failed"),
    }
}

fn authentication(error: &Error) -> bool {
    matches!(error, Error::Database(db) if matches!(db.code().as_deref(), Some("28P01" | "28000")))
}

/// SQLSTATEs whose `message()` names something in the submitted SQL — an
/// absent table or column, a syntax fault, an unknown function, an ambiguous
/// reference, the wrong object kind, a missing privilege, a cardinality fault
/// or a division by zero — and so describes the query rather than stored rows.
///
/// The code is the allow-list key, never the message: an unanticipated code
/// stays redacted, so a message shape this connector never considered cannot
/// leak. `detail()` and `hint()` are never read; the trait object exposes
/// neither, so a unique/constraint fault whose detail echoes a row value is
/// structurally unreachable here.
const EXPLAINABLE: [&str; 10] = [
    "42P01", // undefined_table
    "42703", // undefined_column
    "42601", // syntax_error
    "42883", // undefined_function
    "42702", // ambiguous_column
    "42P10", // invalid_column_reference
    "42809", // wrong_object_type
    "42501", // insufficient_privilege
    "21000", // cardinality_violation
    "22012", // division_by_zero
];

fn classify(code: Option<&str>, message: &str) -> Option<String> {
    let code = code?;
    if !EXPLAINABLE.contains(&code) {
        return None;
    }
    let detail = message.trim();
    if detail.is_empty() {
        return None;
    }
    Some(bounded(detail))
}

const MAX_DETAIL: usize = 300;

fn bounded(message: &str) -> String {
    if message.chars().count() <= MAX_DETAIL {
        return message.to_owned();
    }
    let kept: String = message.chars().take(MAX_DETAIL).collect();
    format!("{kept}…")
}

pub(crate) fn schema(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("PostgreSQL schema discovery failed")
}

pub(crate) fn row(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("PostgreSQL schema result was invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::fmt::{self, Display, Formatter};

    use sqlx::error::{DatabaseError, ErrorKind};

    /// A stand-in for `PgDatabaseError` so the classifier runs without a live
    /// server. `query()` reads only `code()` and `message()` off the trait, so
    /// the stub supplies exactly those; `detail()`/`hint()` are not on the
    /// trait and so cannot be reached.
    #[derive(Debug)]
    struct Fault {
        code: Option<&'static str>,
        message: &'static str,
    }

    impl Display for Fault {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for Fault {}

    impl DatabaseError for Fault {
        fn message(&self) -> &str {
            self.message
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            self.code.map(Cow::Borrowed)
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    fn failure(code: Option<&'static str>, message: &'static str) -> ConnectionError {
        query(Error::from(Fault { code, message }))
    }

    #[test]
    fn missing_table_names_the_table() {
        let text = failure(Some("42P01"), "relation \"orders\" does not exist").to_string();
        assert!(text.contains("orders"), "opaque: {text}");
        assert!(text.contains("does not exist"), "opaque: {text}");
    }

    #[test]
    fn syntax_error_is_forwarded() {
        let text = failure(Some("42601"), "syntax error at or near \"SELEC\"").to_string();
        assert!(text.contains("syntax error"), "opaque: {text}");
        assert!(text.contains("SELEC"), "opaque: {text}");
    }

    #[test]
    fn unanticipated_code_stays_redacted() {
        let text = failure(Some("99999"), "row value 4111-1111-1111-1111").to_string();
        assert!(!text.contains("4111"), "unvetted message leaked: {text}");
        assert_eq!(text, "query failed: PostgreSQL query failed");
    }

    #[test]
    fn missing_code_stays_redacted() {
        let text = failure(None, "row value 4111-1111-1111-1111").to_string();
        assert!(!text.contains("4111"), "codeless message leaked: {text}");
        assert_eq!(text, "query failed: PostgreSQL query failed");
    }

    #[test]
    fn planted_secret_in_excluded_class_does_not_leak() {
        // 23505 unique violation: detail() would echo the row, so the class is
        // excluded. The secret is planted in message() to prove the exclusion
        // drops it rather than forwarding it.
        let text = failure(
            Some("23505"),
            "duplicate key value PLANTED_SECRET violates unique constraint",
        )
        .to_string();
        assert!(!text.contains("PLANTED_SECRET"), "secret leaked: {text}");
        assert_eq!(text, "query failed: PostgreSQL query failed");
    }

    #[test]
    fn excluded_codes_stay_redacted() {
        for code in ["22P02", "22003", "22001", "23505", "23503", "23502"] {
            let text = failure(Some(code), "value PLANTED_SECRET").to_string();
            assert!(
                !text.contains("PLANTED_SECRET"),
                "excluded code {code} leaked: {text}"
            );
            assert_eq!(text, "query failed: PostgreSQL query failed", "code {code}");
        }
    }

    #[test]
    fn authentication_codes_route_to_authentication_failed() {
        for code in ["28P01", "28000"] {
            let text = failure(Some(code), "password PLANTED_SECRET").to_string();
            assert!(
                text.starts_with("authentication failed"),
                "code {code}: {text}"
            );
            assert!(
                !text.contains("PLANTED_SECRET"),
                "code {code} leaked: {text}"
            );
        }
    }
}
