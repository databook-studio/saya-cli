//! Slash-text → `ContractsCommand` translation for the contract slash adapters.
//!
//! This is the *only* new surface in the 2b-4 slice: it turns `/contracts`,
//! `/contract <table>`, `/remember …` and `/forget <id>` into the same
//! [`ContractsCommand`] the headless `saya contracts` clap parser produces, so
//! both paths hand the same typed value to [`crate::commands::run_contracts`].
//! No second parsing of qualified names, no second privacy decision, no second
//! DTO mapping lives here — those all stay inside the shared dispatcher. The
//! parity tests in `tests/contracts_slash_parity.rs` assert the translated
//! command equals the headless one.
//!
//! `/remember` argument shape (the one open question in the spec): the table
//! and kind are positional; for table-scoped kinds everything after the kind is
//! the value (so a description with spaces is natural), and for column-scoped
//! kinds the column is the next positional and everything after it is the
//! value. See [`RememberSpec`] and the SPEC REVIEW.

use crate::cli::{ClaimKindArg, ContractsCommand, ForgetReasonArg, ReviewDecisionArg};
use crate::contracts::args::parse_kind;
use crate::slash::SlashParseError;

/// The fixed shape of a parsed `/remember` request, before it becomes a
/// `ContractsCommand::Remember`. Carried as plain fields so the unit tests can
/// assert the translation without a store.
#[derive(Debug)]
pub(crate) struct RememberSpec {
    pub table: String,
    pub kind: ClaimKindArg,
    pub value: String,
    pub column: Option<String>,
}

/// Kinds whose value binds to a column; the column is the positional after the
/// kind. Matches `build_payload`'s `require_column` arm exactly — the two
/// column kinds and no others.
fn is_column_kind(kind: ClaimKindArg) -> bool {
    matches!(
        kind,
        ClaimKindArg::ColumnDescription | ClaimKindArg::ColumnRole
    )
}

/// Parses a `/remember` argument tail (everything after `/remember `) into a
/// `RememberSpec`. The kind word is the delimiter that splits the tail; an
/// unknown kind is a usage error carrying no untrusted input.
pub(crate) fn parse_remember(arg: &str) -> Result<RememberSpec, SlashParseError> {
    let mut parts = arg.split_whitespace();
    let table = parts
        .next()
        .ok_or_else(|| SlashParseError(usage_remember()))?;
    let kind_word = parts
        .next()
        .ok_or_else(|| SlashParseError(usage_remember()))?;
    let kind = parse_kind(kind_word).ok_or_else(|| SlashParseError(usage_remember()))?;

    let (column, value) = if is_column_kind(kind) {
        let column = parts
            .next()
            .ok_or_else(|| SlashParseError(usage_remember()))?;
        (Some(column.to_string()), rest_after(parts))
    } else {
        (None, rest_after(parts))
    };
    let value = value.ok_or_else(|| SlashParseError(usage_remember()))?;

    Ok(RememberSpec {
        table: table.to_string(),
        kind,
        value,
        column,
    })
}

/// Collects the remaining tokens after the kind (and column, for column kinds)
/// back into the value, collapsing the whitespace the user typed. `None` when
/// nothing remains — a `/remember` with no value is a usage error.
fn rest_after<'a, I: Iterator<Item = &'a str>>(mut parts: I) -> Option<String> {
    let first = parts.next()?;
    let mut value = first.to_string();
    for token in parts {
        value.push(' ');
        value.push_str(token);
    }
    Some(value)
}

/// Payload-free usage for `/remember`. Never echoes the untrusted tail.
fn usage_remember() -> String {
    "/remember <catalog.schema.object> <kind> <value…>\n\
     kinds: description, alias, grain, time-column, column-description <column> <value…>, \
     column-role <column> <role>"
        .into()
}

/// Payload-free usage for `/contract`.
fn usage_contract() -> String {
    "/contract <catalog.schema.object>".into()
}

/// Payload-free usage for `/forget`.
fn usage_forget() -> String {
    "/forget <claim-id>".into()
}

/// Payload-free usage for `/queue`. The limit is optional and numeric.
fn usage_queue() -> String {
    "/queue [limit]".into()
}

/// Payload-free usage for the spec-D decide commands. One token: a stored
/// claim-id prefix (the `ki-xxxx` form `contracts list` abbreviates to). Never
/// echoes the untrusted prefix.
fn usage_decide() -> String {
    "/confirm|/reject|/use <claim-id-prefix>".into()
}

/// Parses the tail of a `/confirm`, `/reject`, or `/use` command: exactly one
/// token (the claim-id prefix). The decision is fixed by the command name.
/// Translates to `ContractsCommand::Decide` with `profile: None` — the TUI
/// stamps the active profile, the headless path resolves the default, exactly
/// as `/queue` does. A too-short prefix is not refused here: the resolve step
/// refuses `c` or empty with a typed message, so parsing stays shape-only.
fn parse_decide(
    _name: &str,
    arg: &str,
    decision: ReviewDecisionArg,
) -> Result<Option<ContractsCommand>, SlashParseError> {
    let tokens: Vec<&str> = arg.split_whitespace().collect();
    let prefix = tokens
        .first()
        .ok_or_else(|| SlashParseError(usage_decide()))?;
    if tokens.len() != 1 {
        return Err(SlashParseError(usage_decide()));
    }
    Ok(Some(ContractsCommand::Decide {
        prefix: prefix.to_string(),
        decision,
        profile: None,
    }))
}

/// Translates a slash command name + its argument tail into the matching
/// `ContractsCommand`, or a usage error. The argument is the raw text after the
/// command word (already trimmed of the leading `/name`).
pub(crate) fn parse_contract_command(
    name: &str,
    arg: &str,
) -> Result<Option<ContractsCommand>, SlashParseError> {
    match name {
        "contracts" => {
            if !arg.trim().is_empty() {
                return Err(SlashParseError("/contracts takes no argument".into()));
            }
            Ok(Some(ContractsCommand::List { profile: None }))
        }
        "contract" => {
            // Exactly one token: the qualified table name. `split_whitespace`
            // already ignores surrounding whitespace, so no leading `trim()`.
            let tokens: Vec<&str> = arg.split_whitespace().collect();
            let table = tokens
                .first()
                .ok_or_else(|| SlashParseError(usage_contract()))?;
            if tokens.len() != 1 {
                return Err(SlashParseError(usage_contract()));
            }
            Ok(Some(ContractsCommand::Show {
                table: table.to_string(),
                profile: None,
            }))
        }
        "remember" => {
            let spec = parse_remember(arg)?;
            Ok(Some(ContractsCommand::Remember {
                table: spec.table,
                kind: spec.kind,
                value: spec.value,
                column: spec.column,
                profile: None,
            }))
        }
        "forget" => {
            // Exactly one token: the claim id.
            let tokens: Vec<&str> = arg.split_whitespace().collect();
            let id = tokens
                .first()
                .ok_or_else(|| SlashParseError(usage_forget()))?;
            if tokens.len() != 1 {
                return Err(SlashParseError(usage_forget()));
            }
            // The default reason matches the headless `forget` default so the
            // translated command equals the headless one with no --reason.
            Ok(Some(ContractsCommand::Forget {
                claim_id: id.to_string(),
                reason: ForgetReasonArg::UserRequest,
            }))
        }
        "queue" => {
            // `/queue` with no args lists the active profile's candidates at the
            // default limit. A single optional positional number overrides the
            // limit. The slash path always uses the active profile — there is
            // no `--profile` form here, matching `/contracts` → `List`.
            let tokens: Vec<&str> = arg.split_whitespace().collect();
            let limit = match tokens.len() {
                0 => None,
                1 => Some(
                    tokens[0]
                        .parse::<usize>()
                        .map_err(|_| SlashParseError(usage_queue()))?,
                ),
                _ => return Err(SlashParseError(usage_queue())),
            };
            Ok(Some(ContractsCommand::Queue {
                profile: None,
                limit,
            }))
        }
        // Spec D: act on the claim the last turn showed, by a short stored
        // claim-id prefix rather than a 64-character id. The decision is fixed
        // by the command name; the prefix is the one positional. See
        // `parse_decide` and the §4 defence in the report.
        "confirm" => parse_decide(name, arg, ReviewDecisionArg::Confirm),
        "reject" => parse_decide(name, arg, ReviewDecisionArg::Reject),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_contract_contracts_no_arg() {
        let cmd = parse_contract_command("contracts", "").unwrap().unwrap();
        assert_eq!(cmd, ContractsCommand::List { profile: None });
        assert!(parse_contract_command("contracts", "x").is_err());
    }

    #[test]
    fn parse_contract_contract_one_table() {
        let cmd = parse_contract_command("contract", "analytics.public.orders")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            ContractsCommand::Show {
                table: "analytics.public.orders".into(),
                profile: None,
            }
        );
        assert!(parse_contract_command("contract", "").is_err());
        assert!(parse_contract_command("contract", "a b").is_err());
    }

    #[test]
    fn parse_remember_table_kind_value() {
        let spec = parse_remember("a.b.c alias customers").unwrap();
        assert_eq!(spec.table, "a.b.c");
        assert_eq!(spec.kind, ClaimKindArg::Alias);
        assert_eq!(spec.value, "customers");
        assert_eq!(spec.column, None);
    }

    #[test]
    fn parse_remember_value_keeps_spaces() {
        let spec = parse_remember("a.b.c description orders fact table").unwrap();
        assert_eq!(spec.kind, ClaimKindArg::Description);
        assert_eq!(spec.value, "orders fact table");
        assert_eq!(spec.column, None);
    }

    #[test]
    fn parse_remember_column_kind_takes_column_then_value() {
        let spec = parse_remember("a.b.c column-description amount order total").unwrap();
        assert_eq!(spec.kind, ClaimKindArg::ColumnDescription);
        assert_eq!(spec.column.as_deref(), Some("amount"));
        assert_eq!(spec.value, "order total");
    }

    #[test]
    fn parse_remember_column_role() {
        let spec = parse_remember("a.b.c column-role amount measure").unwrap();
        assert_eq!(spec.kind, ClaimKindArg::ColumnRole);
        assert_eq!(spec.column.as_deref(), Some("amount"));
        assert_eq!(spec.value, "measure");
    }

    #[test]
    fn parse_remember_time_column_is_table_scoped() {
        let spec = parse_remember("a.b.c time-column created_at").unwrap();
        assert_eq!(spec.kind, ClaimKindArg::TimeColumn);
        assert_eq!(spec.value, "created_at");
        assert_eq!(spec.column, None);
    }

    #[test]
    fn parse_remember_kind_aliases_accepted() {
        // snake_case alias maps to the same variant as the canonical kebab form.
        let spec = parse_remember("a.b.c column_description amount note").unwrap();
        assert_eq!(spec.kind, ClaimKindArg::ColumnDescription);
    }

    #[test]
    fn parse_remember_unknown_kind_is_usage_error_without_echo() {
        let bad = parse_remember("a.b.c not-a-kind value").unwrap_err();
        assert!(!bad.0.contains("not-a-kind"));
        assert!(!bad.0.contains("a.b.c"));
        assert!(bad.0.contains("kind"));
    }

    #[test]
    fn parse_remember_too_few_args_is_usage_error() {
        assert!(parse_remember("").is_err());
        assert!(parse_remember("a.b.c").is_err());
        assert!(parse_remember("a.b.c alias").is_err());
        // column kind with no column.
        assert!(parse_remember("a.b.c column-description").is_err());
        assert!(parse_remember("a.b.c column-description amount").is_err());
    }

    #[test]
    fn parse_forget_one_id() {
        let cmd = parse_contract_command("forget", "abc-123")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            ContractsCommand::Forget {
                claim_id: "abc-123".into(),
                reason: ForgetReasonArg::UserRequest,
            }
        );
        assert!(parse_contract_command("forget", "").is_err());
        assert!(parse_contract_command("forget", "a b").is_err());
    }

    #[test]
    fn parse_queue_no_arg_is_default_limit() {
        let cmd = parse_contract_command("queue", "").unwrap().unwrap();
        assert_eq!(
            cmd,
            ContractsCommand::Queue {
                profile: None,
                limit: None
            }
        );
    }

    #[test]
    fn parse_queue_optional_numeric_limit() {
        let cmd = parse_contract_command("queue", "20").unwrap().unwrap();
        assert_eq!(
            cmd,
            ContractsCommand::Queue {
                profile: None,
                limit: Some(20),
            }
        );
        // A non-numeric limit is a usage error that does not echo the input.
        let bad = parse_contract_command("queue", "lots").unwrap_err();
        assert!(!bad.0.contains("lots"));
        // Two tokens is a usage error.
        assert!(parse_contract_command("queue", "1 2").is_err());
    }

    #[test]
    fn parse_confirm_one_prefix() {
        let cmd = parse_contract_command("confirm", "c-a86a3f")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            ContractsCommand::Decide {
                prefix: "c-a86a3f".into(),
                decision: ReviewDecisionArg::Confirm,
                profile: None,
            }
        );
        // No token is a usage error that does not echo anything.
        let bad = parse_contract_command("confirm", "").unwrap_err();
        assert!(!bad.0.contains("c-"));
        assert!(!bad.0.is_empty());
        // Two tokens is a usage error that does not echo the input.
        let bad = parse_contract_command("confirm", "c-abc c-def").unwrap_err();
        assert!(!bad.0.contains("c-abc"));
        assert!(!bad.0.contains("c-def"));
    }

    #[test]
    fn parse_reject_translates_to_decide() {
        assert_eq!(
            parse_contract_command("reject", "c-1").unwrap().unwrap(),
            ContractsCommand::Decide {
                prefix: "c-1".into(),
                decision: ReviewDecisionArg::Reject,
                profile: None,
            }
        );
        // Refuses a missing prefix without panicking.
        assert!(parse_contract_command("reject", "").is_err());
        // Refuses two tokens.
        assert!(parse_contract_command("reject", "a b").is_err());
    }

    /// `/use` is deliberately absent. `use_candidate_once` validates a claim but
    /// the interactive session does not yet thread the admission into the next
    /// recall, so the command could not do what its name promises. Shipping it
    /// would have told a user their candidate was admitted when nothing had
    /// changed — the exact overstatement this feature exists to avoid. It
    /// returns here once the admission is threaded.
    #[test]
    fn use_is_not_a_contract_command_until_the_admission_is_threaded() {
        assert!(
            parse_contract_command("use", "c-1").unwrap().is_none(),
            "/use must not parse while it cannot admit anything"
        );
    }

    #[test]
    fn parse_unknown_name_returns_none() {
        assert!(parse_contract_command("sql", "select 1").unwrap().is_none());
    }
}
