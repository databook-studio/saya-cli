//! Help text for the slash commands: the one-line summary (`/help` with no
//! argument) and the per-command usage + example (`/help <command>`).
//!
//! Extracted from `slash.rs` to keep that file under the size cap. `slash.rs`
//! re-exports [`help_for`] so the existing `crate::slash::help_for` path used by
//! the session command layer keeps resolving.

/// The one-line summary printed by `/help` with no argument. One string so the
/// TUI and headless paths render it identically.
pub(crate) fn help_text() -> &'static str {
    "/connect <profile>  /connections  /include <profile>  /exclude <profile>\n/provider [name]     /model [name]  /privacy [on|off]\n/approvals [ask|read-only|never]  /schema [refresh]  /sql <query>  /export <path>\n/explain [sql]  /clear  /history  /sessions  /resume <id>  /doctor  /help  /exit\n/contracts  /contract <table>  /remember <table> <kind> <value…>  /forget <id>  /queue [limit]\n/confirm <prefix>  /reject <prefix> — act on a claim shown this turn by its short id prefix"
}

/// Returns a short usage and example string for a known slash command, or `None` if unknown.
pub(crate) fn command_help(name: &str) -> Option<&'static str> {
    let clean_name = name.trim_start_matches('/').to_lowercase();
    match clean_name.as_str() {
        "connect" => {
            Some("connect <profile> — set the active database profile. Example: /connect prod")
        }
        "connections" => Some(
            "connections — list configured database connection profiles. Example: /connections",
        ),
        "include" => Some(
            "include <profile> — include an additional database profile. Example: /include staging",
        ),
        "exclude" => {
            Some("exclude <profile> — exclude a database profile. Example: /exclude staging")
        }
        "provider" => {
            Some("provider [name] — view or set the AI provider. Example: /provider anthropic")
        }
        "model" => Some("model [name] — view or set the AI model. Example: /model gpt-4o"),
        "privacy" => {
            Some("privacy [on|off] — view or toggle cloud data sharing. Example: /privacy off")
        }
        "approvals" => Some(
            "approvals [ask|read-only|never] — view or set tool execution approval policy. Example: /approvals ask",
        ),
        "schema" => Some(
            "schema [refresh] — display or refresh database schema context. Example: /schema refresh",
        ),
        "sql" => Some(
            "sql <query> — execute a raw SQL query directly. Example: /sql SELECT * FROM users LIMIT 10;",
        ),
        "export" => Some(
            "export <path> — write the last query's rows to a .csv or .json file. Example: /export results.csv",
        ),
        "chart" => Some(
            "chart [type] [path] — render the last query as an interactive HTML chart and open it. type: bar|line|area|pie|doughnut|scatter (default auto)",
        ),
        "explain" => Some(
            "explain [sql] — show the query plan (EXPLAIN) for the given SQL, or the last query if omitted",
        ),
        "clear" => Some("clear — clear the conversation and context. Example: /clear"),
        "history" => Some("history — list saved sessions as text. Example: /history"),
        "sessions" => {
            Some("sessions — browse saved sessions; opens a picker in the TUI. Example: /sessions")
        }
        "doctor" => Some(
            "doctor — diagnose configuration: secrets resolve? provider endpoint? Example: /doctor",
        ),
        "resume" => Some("resume <id> — resume a previous session by ID. Example: /resume 12345"),
        "contracts" => {
            Some("contracts — list recalled contracts for the active profile. Example: /contracts")
        }
        "contract" => Some(
            "contract <catalog.schema.object> — show one object's contract. Example: /contract analytics.public.orders",
        ),
        "remember" => Some(
            "remember <catalog.schema.object> <kind> <value…> [because <reason…>] — store a confirmed claim. Kinds: description, alias, grain, time-column, column-description <column> <value…>, column-role <column> <role>. The optional `because <reason…>` (directive kinds only) records why the claim holds. Example: /remember analytics.public.orders time-column created_at because orders complete on return",
        ),
        "forget" => Some(
            "forget <claim-id> — tombstone a claim so recall excludes it. Example: /forget abc-123",
        ),
        "queue" => {
            Some("queue [limit] — list candidate claims awaiting review. Example: /queue 20")
        }
        "confirm" => Some(
            "confirm <claim-id-prefix> — confirm the claim named by its short id prefix (the ki-xxxx form /contracts shows). Example: /confirm ki-a86a3f",
        ),
        "reject" => Some(
            "reject <claim-id-prefix> — reject the claim named by its short id prefix. Example: /reject ki-a86a3f",
        ),
        "help" => Some(
            "help [command] — display general help or detailed usage for a command. Example: /help connect",
        ),
        "exit" | "quit" => Some("exit — exit the interactive CLI session. Example: /exit"),
        _ => None,
    }
}

/// Returns command-specific help for a topic, or general help text if `topic` is `None`.
pub(crate) fn help_for(topic: Option<&str>) -> String {
    match topic {
        Some(name) => {
            let clean = name.trim_start_matches('/');
            match command_help(clean) {
                Some(help) => help.to_string(),
                None => format!("No help for /{clean}. Type /help for the full list."),
            }
        }
        None => help_text().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/history` and `/sessions` are **not** interchangeable, and the help
    /// must not claim they are. They share one `SessionAction` in the headless
    /// REPL, but `tui/dispatch.rs` intercepts the literal line `/sessions`
    /// before the parser and opens the interactive picker, so in the TUI —
    /// the primary surface — `/sessions` is a picker and `/history` is a text
    /// list. Help that called one an alias of the other would be wrong exactly
    /// where most people read it. Both name "saved sessions" so neither implies
    /// the conversation or the input-line history.
    #[test]
    fn history_and_sessions_help_describe_their_real_surfaces() {
        let history = command_help("history").expect("history has help");
        let sessions = command_help("sessions").expect("sessions has help");

        // Neither may claim to be an alias of the other: the TUI behaviours differ
        assert!(
            !history.contains("alias") && !sessions.contains("alias"),
            "neither may claim aliasing — the TUI routes them differently: {history} / {sessions}"
        );
        // `/sessions` must mention the picker, which is what the TUI does.
        assert!(
            sessions.contains("picker"),
            "`/sessions` help must mention the picker it opens in the TUI, got: {sessions}"
        );
        // Both are about sessions saved on disk, not the conversation and not
        // the input-line history.
        assert!(
            history.contains("saved") && sessions.contains("saved"),
            "both must name saved sessions, got: {history} / {sessions}"
        );
    }

    /// transcript; "conversation and context" is accurate and unambiguous.
    #[test]
    fn clear_help_describes_the_conversation_not_history() {
        let clear = command_help("clear").expect("clear has help");
        assert!(
            clear.contains("conversation") && clear.contains("context"),
            "/clear help should describe the conversation and context, got: {clear}"
        );
        assert!(
            !clear.contains("history"),
            "/clear help must not reuse the overloaded 'history' word (now = saved sessions), got: {clear}"
        );
    }
}
