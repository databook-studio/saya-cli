use saya_types::ConnectionError;
use sqlx::Error;

pub(crate) fn connection(_: Error) -> ConnectionError {
    ConnectionError::connection_failed("SQLite connection failed")
}

pub(crate) fn query(error: Error) -> ConnectionError {
    match sql_name_fault(&error) {
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

/// The faults that are about a *name in the submitted SQL* rather than about
/// any stored value.
///
/// Driver text is not passed through. Each entry matches one SQLite phrasing
/// and the message is rebuilt from the identifier that follows it, so an
/// unanticipated message shape yields `None` and stays redacted. The identifier
/// itself came from the caller's own SQL, so returning it discloses nothing the
/// caller did not already hold — while telling it whether the name was a
/// function, a table or a column, which is the difference between correcting
/// the query and guessing at it.
fn sql_name_fault(error: &Error) -> Option<String> {
    const PREFIXES: [(&str, &str); 4] = [
        ("no such function: ", "no such function"),
        ("no such table: ", "no such table"),
        ("no such column: ", "no such column"),
        ("ambiguous column name: ", "ambiguous column name"),
    ];

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
