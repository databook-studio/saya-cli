//! The session-grant scope sentence: what accepting `[s]` widens to, in
//! words, before the user presses it. A scope sentence renders only when a
//! session grant is actually on offer (`grant` is `Some`) — never a grant
//! the user is not being offered — and only for the token the grant names:
//! the sentence describes the token, never a second reading of the call.
//!
//! The sentences are wording only: they name the grammar's own scope
//! (`grant_token.rs` is deliberately coarse) so the token stops reading as
//! an explanation. They change nothing a grant permits, which keys answer,
//! or when a grant is offered.

/// The scope sentence for the offered session grant, when this family has
/// one. `name` is the tool the ask is for; `grant` is the token actually on
/// offer — `None` renders no sentence, ever. The sentence names only the
/// offered token's scope:
///
/// - `run_command` / `run_program`: the token covers that program with any
///   arguments for the session.
/// - fetch tools: the token covers that scheme and host, any path.
/// - `workspace-write`: the token covers every workspace write, not this
///   one path.
///
/// Any other token — `sql:`, `scratch`, a family this sentence does not
/// name — renders no sentence: the sentence appears only where the packet
/// wrote one, never a grant the wording was not built for.
pub(super) fn scope_sentence(name: &str, grant: Option<&str>) -> Option<String> {
    let token = grant?;
    match name {
        "run_command" => {
            let program = token.strip_prefix("command:")?;
            Some(format!(
                "  [s] covers {program} with any arguments for this session — not just this argv"
            ))
        }
        "run_program" => {
            let program = token
                .strip_prefix("runner:")
                .or_else(|| token.strip_prefix("interpreter:"))?;
            Some(format!(
                "  [s] covers {program} with any arguments for this session — not just this argv"
            ))
        }
        "http_fetch" | "http_download" => {
            let destination = token.strip_prefix("fetch:")?;
            let (scheme, host) = destination.split_once('+')?;
            Some(format!(
                "  [s] covers {scheme} on {host}, any path, for this session — not just this URL"
            ))
        }
        "workspace_write" if token == "workspace-write" => Some(
            "  [s] covers every workspace write for this session — not this path alone".to_string(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::scope_sentence;

    #[test]
    fn no_grant_offered_means_no_scope_sentence() {
        for name in [
            "run_command",
            "run_program",
            "http_fetch",
            "http_download",
            "workspace_write",
        ] {
            assert_eq!(scope_sentence(name, None), None);
        }
    }

    #[test]
    fn a_scope_sentence_never_names_a_token_outside_its_family() {
        assert_eq!(scope_sentence("run_command", Some("sql:analytics")), None);
        assert_eq!(scope_sentence("http_fetch", Some("workspace-write")), None);
        assert_eq!(
            scope_sentence("workspace_write", Some("fetch:https+example.com")),
            None
        );
        assert_eq!(
            scope_sentence("bounded_sql_query", Some("sql:analytics")),
            None
        );
        assert_eq!(scope_sentence("scratch_sql", Some("scratch")), None);
    }
}
