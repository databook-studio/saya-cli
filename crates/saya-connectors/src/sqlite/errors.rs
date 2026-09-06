use saya_types::ConnectionError;
use sqlx::Error;

pub(crate) fn connection(_: Error) -> ConnectionError {
    ConnectionError::connection_failed("SQLite connection failed")
}

pub(crate) fn query(error: Error) -> ConnectionError {
    match explanation(&error) {
        Some(reason) => ConnectionError::query_failed(format!("SQLite query failed: {reason}")),
        None => ConnectionError::query_failed("SQLite query failed"),
    }
}

pub(crate) fn schema(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("SQLite schema discovery failed")
}

pub(crate) fn row(_: Error) -> ConnectionError {
    ConnectionError::schema_failed("SQLite schema result was invalid")
}

/// Faults that are about a *name in the submitted SQL* or a constant structural
/// fault, never about a stored value.
///
/// Driver text is not passed through. Each entry in `PREFIXES` matches one
/// SQLite phrasing and the message is rebuilt from the identifier that follows
/// it, so an unanticipated shape yields `None` and stays redacted. The
/// identifier itself came from the caller's own SQL, so returning it discloses
/// nothing the caller did not already hold — while telling it whether the name
/// was a function, a table or a column, which is the difference between
/// correcting the query and guessing at it. The entries in `CONSTANTS` carry
/// no interpolated value at all and are forwarded verbatim, bounded.
fn explanation(error: &Error) -> Option<String> {
    const PREFIXES: [(&str, &str); 6] = [
        ("no such function: ", "no such function"),
        ("no such table: ", "no such table"),
        ("no such column: ", "no such column"),
        ("ambiguous column name: ", "ambiguous column name"),
        ("near \"", "syntax error"),
        (
            "wrong number of arguments to function ",
            "wrong number of arguments to function",
        ),
    ];
    const CONSTANTS: [&str; 2] = ["datatype mismatch", "sub-select returns"];

    let text = match error {
        Error::Database(db) => db.message().to_string(),
        _ => return None,
    };

    for (needle, label) in PREFIXES {
        if let Some(rest) = text.strip_prefix(needle) {
            let name = identifier(rest)?;
            return Some(format!("{label}: {name}"));
        }
    }
    for needle in CONSTANTS {
        if text.starts_with(needle) {
            return Some(bounded(&text));
        }
    }
    None
}

/// The identifier SQLite names, bounded and validated. Only a plain SQL name
/// (letters, digits, `_`, `$`, and `.` for a qualified one) is returned, so
/// nothing that could carry a value or a fragment of a row travels with it.
fn identifier(rest: &str) -> Option<String> {
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
        .take(64)
        .collect();
    (!name.is_empty()).then_some(name)
}

const MAX_DETAIL: usize = 300;

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
    use std::fmt::{self, Display, Formatter};

    use sqlx::error::{DatabaseError, ErrorKind};

    /// A stand-in for `SqliteError` so the classifier runs without a live
    /// database. `explanation()` reads only `message()` off the trait, so the
    /// stub supplies exactly that.
    #[derive(Debug)]
    struct Fault {
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

    fn failure(message: &'static str) -> ConnectionError {
        query(Error::from(Fault { message }))
    }

    #[test]
    fn missing_table_names_the_table() {
        let text = failure("no such table: orders").to_string();
        assert!(text.contains("no such table: orders"), "opaque: {text}");
    }

    #[test]
    fn syntax_error_names_the_token() {
        let text = failure("near \"SELEC\": syntax error").to_string();
        assert!(text.contains("syntax error: SELEC"), "opaque: {text}");
    }

    #[test]
    fn wrong_argument_count_names_the_function() {
        let text = failure("wrong number of arguments to function round()").to_string();
        assert!(
            text.contains("wrong number of arguments to function: round"),
            "opaque: {text}"
        );
    }

    #[test]
    fn datatype_mismatch_is_forwarded_verbatim() {
        let text = failure("datatype mismatch").to_string();
        assert!(text.contains("datatype mismatch"), "opaque: {text}");
    }

    #[test]
    fn sub_select_returns_is_forwarded_verbatim() {
        let text = failure("sub-select returns more than one row").to_string();
        assert!(
            text.contains("sub-select returns more than one row"),
            "opaque: {text}"
        );
    }

    #[test]
    fn unanticipated_message_stays_redacted() {
        let text = failure("database disk image is malformed").to_string();
        assert_eq!(text, "query failed: SQLite query failed");
    }

    #[test]
    fn planted_secret_in_excluded_class_does_not_leak() {
        // A constraint fault echoes the stored value; it matches no recognised
        // prefix, so the message is dropped rather than forwarded.
        let text = failure("UNIQUE constraint failed: t.x = 'PLANTED_SECRET'").to_string();
        assert!(!text.contains("PLANTED_SECRET"), "secret leaked: {text}");
        assert_eq!(text, "query failed: SQLite query failed");
    }
}
