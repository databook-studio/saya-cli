//! How a grant is said to the user: its session-history family and the
//! answers line both approval frontends render.

/// The session-history family a granted token belongs to — the prefix group
/// the grammar's tokens come in (`sql:`, `runner:`/`interpreter:`,
/// `fetch:`), or the tool's own bare token. A prompt's session-history line
/// shows the held grants of the call's family, in these tokens' own words;
/// a token outside every family shows in none. The words are the store's
/// tokens verbatim — this names families, never re-spells tokens.
pub(crate) fn grant_family(token: &str) -> Option<&'static str> {
    if token.starts_with("sql:") {
        Some("SQL")
    } else if token.starts_with("runner:") || token.starts_with("interpreter:") {
        Some("runner")
    } else if token.starts_with("fetch:") {
        Some("fetch")
    } else if token.starts_with("command:") {
        Some("command")
    } else if token == "workspace-write" {
        Some("workspace-write")
    } else if token == "scratch" {
        Some("scratch")
    } else {
        None
    }
}

/// The answers line both approval frontends render: the third answer names
/// the token verbatim when one exists, and with none the line offers the two
/// answers and says so — it must never offer a grant it cannot name.
pub(crate) fn session_answers_line(grant: Option<&str>) -> String {
    match grant {
        Some(token) => format!("[a] allow once   [s] allow {token} for this session   [d] deny"),
        None => "[a] allow once   [d] deny   (no session grant for this tool)".to_owned(),
    }
}
