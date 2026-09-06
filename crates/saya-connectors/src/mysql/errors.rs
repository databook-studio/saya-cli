use saya_types::ConnectionError;

pub(crate) fn connection(error: sqlx::Error) -> ConnectionError {
    if authentication(&error) {
        ConnectionError::authentication_failed("MySQL authentication failed")
    } else {
        ConnectionError::connection_failed("MySQL connection failed")
    }
}

pub(crate) fn query(error: sqlx::Error) -> ConnectionError {
    if authentication(&error) {
        return ConnectionError::authentication_failed("MySQL authentication failed");
    }
    match error {
        sqlx::Error::Database(db) => {
            // MySQL uses SQLSTATE as a coarse category and the error number for
            // granularity, so the number is the allow-list key. If the downcast
            // somehow fails, `number` is `None` and the message stays redacted.
            let number = db
                .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                .map(sqlx::mysql::MySqlDatabaseError::number);
            match classify(number, db.message()) {
                Some(detail) => {
                    ConnectionError::query_failed(format!("MySQL query failed: {detail}"))
                }
                None => ConnectionError::query_failed("MySQL query failed"),
            }
        }
        _ => ConnectionError::query_failed("MySQL query failed"),
    }
}

pub(crate) fn schema(_: sqlx::Error) -> ConnectionError {
    ConnectionError::schema_failed("MySQL schema discovery failed")
}

fn authentication(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if matches!(database.code().as_deref(), Some("1045" | "28000")))
}

/// MySQL error numbers whose `message()` describes the submitted statement —
/// an absent table, column, function or database, a parse error, an ambiguous
/// column, an only-full-group-by fault, an operand-count or subquery-count
/// fault, or an order-by not in the select list — rather than stored rows.
///
/// The number is the allow-list key, never the message: an unanticipated
/// number stays redacted. `MySqlDatabaseError` exposes no `detail()`/`hint()`,
/// and conversion/truncation/constraint numbers (`1292`, `1366`, `1406`,
/// `1264`, `1062`) are excluded because their messages echo stored values.
const EXPLAINABLE: [u16; 12] = [
    1146, // ER_NO_SUCH_TABLE
    1054, // ER_BAD_FIELD_ERROR
    1064, // ER_PARSE_ERROR
    1052, // ER_NONUNQ_TABLE
    1305, // ER_SP_DOES_NOT_EXIST
    1109, // ER_UNKNOWN_TABLE
    1049, // ER_BAD_DB_ERROR
    1055, // ER_WRONG_FIELD_WITH_GROUP
    1140, // ER_MIX_OF_GROUP_FUNC_AND_FIELDS
    1241, // ER_OPERAND_COLUMNS
    1242, // ER_SUBQUERY_NO_1_ROW
    3065, // ER_ORDER_BY_IN_SELECT_LIST
];

fn classify(number: Option<u16>, message: &str) -> Option<String> {
    let number = number?;
    if !EXPLAINABLE.contains(&number) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::fmt::{self, Display, Formatter};

    use sqlx::error::{DatabaseError, ErrorKind};

    /// A stand-in for `MySqlDatabaseError` so the classifier runs without a
    /// live server. The real type's constructor is crate-private; the stub
    /// supplies `code()` (for the auth check) and `message()`. It does not
    /// downcast to `MySqlDatabaseError`, so `query()` sees `number = None` and
    /// redacts — which is exactly the fail-closed path the classifier takes on
    /// any error it cannot identify.
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

    #[test]
    fn missing_table_names_the_table() {
        let detail = classify(Some(1146), "Table 'db.orders' doesn't exist").unwrap();
        assert!(detail.contains("orders"), "opaque: {detail}");
        assert!(detail.contains("doesn't exist"), "opaque: {detail}");
    }

    #[test]
    fn syntax_error_is_forwarded() {
        let detail = classify(
            Some(1064),
            "You have an error in your SQL syntax near 'SELEC'",
        )
        .unwrap();
        assert!(detail.contains("SQL syntax"), "opaque: {detail}");
        assert!(detail.contains("SELEC"), "opaque: {detail}");
    }

    #[test]
    fn unanticipated_number_stays_redacted() {
        assert_eq!(classify(Some(9999), "row value 4111-1111-1111-1111"), None);
    }

    #[test]
    fn missing_number_stays_redacted() {
        assert_eq!(classify(None, "row value 4111-1111-1111-1111"), None);
    }

    #[test]
    fn planted_secret_in_excluded_class_does_not_leak() {
        // 1062 duplicate entry echoes the stored value, so it is excluded.
        let result = classify(
            Some(1062),
            "Duplicate entry 'PLANTED_SECRET' for key 'users.email'",
        );
        assert!(result.is_none(), "excluded number forwarded: {result:?}");
    }

    #[test]
    fn excluded_numbers_stay_redacted() {
        for number in [1292, 1366, 1406, 1264, 1062] {
            let result = classify(Some(number), "value PLANTED_SECRET");
            assert!(
                result.is_none(),
                "excluded number {number} forwarded: {result:?}"
            );
        }
    }

    #[test]
    fn authentication_codes_route_to_authentication_failed() {
        for code in ["1045", "28000"] {
            let text = query(sqlx::Error::from(Fault {
                code: Some(code),
                message: "password PLANTED_SECRET",
            }))
            .to_string();
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

    #[test]
    fn undowncastable_database_error_stays_redacted() {
        // A database error that is not a MySqlDatabaseError cannot yield a
        // number, so the classifier fails closed rather than guessing.
        let text = query(sqlx::Error::from(Fault {
            code: Some("00000"),
            message: "value PLANTED_SECRET",
        }))
        .to_string();
        assert!(!text.contains("PLANTED_SECRET"), "leaked: {text}");
        assert_eq!(text, "query failed: MySQL query failed");
    }

    #[test]
    fn non_database_error_stays_redacted() {
        let text = query(sqlx::Error::Protocol("value PLANTED_SECRET".into())).to_string();
        assert!(!text.contains("PLANTED_SECRET"), "leaked: {text}");
        assert_eq!(text, "query failed: MySQL query failed");
    }
}
