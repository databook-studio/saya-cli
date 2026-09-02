//! Slash-text → `ContractsCommand` translation for the contract slash adapters.
//!
//! This is the *only* new surface in the 2b-4 slice: it turns `/contracts
//! [table]`, `/remember …` and `/forget <id>` into the same
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
    /// An optional reason a directive claim carries, stated as a `because …`
    /// suffix. `None` when the user stated no reason — the common case.
    pub reason: Option<String>,
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
///
/// A reason may be stated as a trailing `because <reason…>` clause: the first
/// standalone `because` token splits the value from the reason, so a user
/// writes `/remember pagila.public.rental time-column return_date because a
/// rental only counts once it comes back`. A value with no `because` carries
/// no reason. The clause is only forwarded to directive kinds (grain,
/// time-column, column-role); for description/alias it is left on the value,
/// matching the headless `--reason` which is ignored there too — though a user
/// who meant a literal "because" in a description should use `--value` to keep
/// it unambiguous.
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
    let raw_value = value.ok_or_else(|| SlashParseError(usage_remember()))?;
    let (value, reason) = split_reason(&raw_value, kind);

    Ok(RememberSpec {
        table: table.to_string(),
        kind,
        value,
        column,
        reason,
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

/// Splits a trailing `because <reason…>` clause off the value for a directive
/// kind. The first standalone `because` token (case-insensitive) is the
/// separator: the text before it is the value, the text after is the reason.
/// For a non-directive kind (description/alias) the value is returned whole
/// and no reason is split — a description legitimately contains "because", and
/// the directive constructors are the only ones that accept a reason. `None`
/// reason when there is no `because` token.
fn split_reason(value: &str, kind: ClaimKindArg) -> (String, Option<String>) {
    if !is_directive_kind(kind) {
        return (value.to_string(), None);
    }
    // Find the first standalone `because` token, case-insensitive. A token is
    // standalone when it is bounded by whitespace or the string ends — so
    // "because" mid-word (e.g. "probecause") is not a split. The value is
    // already whitespace-collapsed by `rest_after`, so a space on both sides (or
    // a leading "because ") is the delimiter.
    let lower = value.to_ascii_lowercase();
    let Some(idx) = find_standalone(&lower, "because") else {
        return (value.to_string(), None);
    };
    let reason = value[idx + "because".len()..].trim();
    let value_part = value[..idx].trim_end();
    if reason.is_empty() || value_part.is_empty() {
        // An empty reason or an empty value after the split means the `because`
        // was not a real clause — treat the whole thing as the value.
        return (value.to_string(), None);
    }
    (value_part.to_string(), Some(reason.to_string()))
}

/// True for the directive kinds that carry a reason: grain, time-column, and
/// column-role. Matches the constructors `build_payload` forwards `reason`
/// to. Description and alias are prose, not directives, and take no reason.
fn is_directive_kind(kind: ClaimKindArg) -> bool {
    matches!(
        kind,
        ClaimKindArg::Grain | ClaimKindArg::TimeColumn | ClaimKindArg::ColumnRole
    )
}

/// Finds the byte offset of the first standalone occurrence of `needle` in
/// `haystack` (already case-folded), where "standalone" means preceded by the
/// start of the string or a space, and followed by the end or a space. Returns
/// `None` when `needle` appears only as a substring of a larger word.
fn find_standalone(haystack: &str, needle: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(idx) = haystack[start..].find(needle) {
        let abs = start + idx;
        let before_ok = abs == 0 || haystack.as_bytes().get(abs - 1) == Some(&b' ');
        let after = abs + needle.len();
        let after_ok = after >= haystack.len() || haystack.as_bytes().get(after) == Some(&b' ');
        if before_ok && after_ok {
            return Some(abs);
        }
        start = abs + needle.len();
    }
    None
}

/// Payload-free usage for `/remember`. Never echoes the untrusted tail.
fn usage_remember() -> String {
    "/remember <catalog.schema.object> <kind> <value…> [because <reason…>]\n\
     kinds: description, alias, grain, time-column, column-description <column> <value…>, \
     column-role <column> <role>\n\
     `because <reason…>` is optional, and only the directive kinds (grain, time-column, \
     column-role) carry it"
        .into()
}

/// Payload-free usage for the merged `/contracts` command. Never echoes the
/// untrusted tail — a two-token argument is a usage error, not a Show of either
/// token. The optional argument is what selects the operation: absent → list,
/// one token → show.
fn usage_contracts() -> String {
    "/contracts [catalog.schema.object]".into()
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

/// Payload-free usage for `/approve-all`. The optional tokens are
/// `--yes` and a numeric limit; anything else is a usage error.
fn usage_approve_all() -> String {
    "/approve-all [--yes] [limit]".into()
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
        // one command, the optional argument selects the operation — what
        // the headless `saya contracts` CLI already does (`contracts list`,
        // `contracts show <t>`), and the same shape as `/queue [limit]`. No
        // argument → list every contract for the active profile; one token →
        // show that object's contract; two tokens → a payload-free usage error
        // (a qualified name is a single token). `/contract` is kept as a silent
        // alias of this same arm so the two names can no longer disagree on
        // what they do — see `contract_is_a_silent_alias_of_the_merged_command`
        // and the SPEC REVIEW for why it is an alias rather than removed. Both
        // pass `profile: None`: the TUI stamps the active profile, the
        // headless path resolves the default, exactly as before.
        "contracts" | "contract" => {
            let tokens: Vec<&str> = arg.split_whitespace().collect();
            match tokens.len() {
                0 => Ok(Some(ContractsCommand::List { profile: None })),
                1 => Ok(Some(ContractsCommand::Show {
                    table: tokens[0].to_string(),
                    profile: None,
                })),
                _ => Err(SlashParseError(usage_contracts())),
            }
        }
        "remember" => {
            let spec = parse_remember(arg)?;
            Ok(Some(ContractsCommand::Remember {
                table: spec.table,
                kind: spec.kind,
                value: spec.value,
                column: spec.column,
                reason: spec.reason,
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
        // the batch approve. `/approve-all` previews the queue and
        // approves nothing — deny by default, the same posture
        // `--non-interactive` gives approvals — and `/approve-all --yes`
        // approves. An optional number overrides the limit, matching
        // `/queue [limit]`. `profile: None`: the TUI stamps the active
        // profile, the headless path resolves the default.
        "approve-all" => {
            let mut yes = false;
            let mut limit = None;
            for token in arg.split_whitespace() {
                if token == "--yes" {
                    if yes {
                        return Err(SlashParseError(usage_approve_all()));
                    }
                    yes = true;
                } else if let Ok(parsed) = token.parse::<usize>() {
                    if limit.is_some() {
                        return Err(SlashParseError(usage_approve_all()));
                    }
                    limit = Some(parsed);
                } else {
                    return Err(SlashParseError(usage_approve_all()));
                }
            }
            Ok(Some(ContractsCommand::ApproveAll {
                profile: None,
                limit,
                yes,
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the trap that was removed, and the regression guard against its
    /// return. Before the merge, `/contracts` and `/contract` were two *separate
    /// operations* distinguished only by a trailing `s`: `/contracts` was `List`
    /// and rejected any argument, `/contract` was `Show` and rejected none. A
    /// user who mistyped the one letter they would not notice got a usage error
    /// from the command they did not mean, and `closest_command` resolved the
    /// near-miss to the *other* command — so it could not help. The before state
    /// was captured by an earlier form of this test that asserted the asymmetry;
    /// it passed against the unmerged code and failed once the merge landed,
    /// proving the behaviour changed exactly as intended. This is the forward
    /// guard: the one-letter difference must no longer select a different
    /// operation or turn a valid argument into a usage error.
    #[test]
    fn s13_the_one_letter_no_longer_selects_a_different_operation() {
        // `/contracts <table>` is now Show, not the "takes no argument" error the
        // old List-only spelling raised — so a user who meant Show no longer gets
        // a usage error for supplying the very argument Show needs.
        assert_eq!(
            parse_contract_command("contracts", "analytics.public.orders")
                .unwrap()
                .unwrap(),
            ContractsCommand::Show {
                table: "analytics.public.orders".into(),
                profile: None,
            }
        );
        // `/contract` with no argument is now List, not the "missing argument"
        // error the old Show-only spelling raised — so a user who meant List no
        // longer gets a usage error for omitting the argument List takes none of.
        assert_eq!(
            parse_contract_command("contract", "").unwrap().unwrap(),
            ContractsCommand::List { profile: None }
        );
        // The two names no longer map to two different operations: for every
        // argument shape they produce the *same* command, so mistyping the one
        // letter cannot land you in a different operation.
        assert_eq!(
            parse_contract_command("contracts", "").unwrap().unwrap(),
            parse_contract_command("contract", "").unwrap().unwrap()
        );
        assert_eq!(
            parse_contract_command("contracts", "analytics.public.orders")
                .unwrap()
                .unwrap(),
            parse_contract_command("contract", "analytics.public.orders")
                .unwrap()
                .unwrap()
        );
    }

    #[test]
    fn parse_contract_contracts_no_arg() {
        let cmd = parse_contract_command("contracts", "").unwrap().unwrap();
        assert_eq!(cmd, ContractsCommand::List { profile: None });
        // After the merge, `/contracts <table>` is Show, not an error.
        assert_eq!(
            parse_contract_command("contracts", "analytics.public.orders")
                .unwrap()
                .unwrap(),
            ContractsCommand::Show {
                table: "analytics.public.orders".into(),
                profile: None,
            }
        );
        // Two tokens is still a usage error: a qualified name is one token.
        assert!(parse_contract_command("contracts", "a b").is_err());
    }

    /// one name, the optional argument selects the operation —
    /// what the CLI already does, and the same shape as `/queue [limit]`. No
    /// argument is List; one token is Show. The trap is gone because there is
    /// no second name to mistype into a different operation.
    #[test]
    fn contracts_one_name_arg_selects_the_operation() {
        // No argument → list every contract for the active profile.
        assert_eq!(
            parse_contract_command("contracts", "").unwrap().unwrap(),
            ContractsCommand::List { profile: None }
        );
        // One token → show that one object's contract.
        assert_eq!(
            parse_contract_command("contracts", "analytics.public.orders")
                .unwrap()
                .unwrap(),
            ContractsCommand::Show {
                table: "analytics.public.orders".into(),
                profile: None,
            }
        );
        // Two tokens is a usage error — a qualified name is a single token, and a
        // Show never takes two. The error is payload-free (no echo of the input).
        let bad = parse_contract_command("contracts", "a b").unwrap_err();
        assert!(!bad.0.contains("a b"));
        assert!(!bad.0.is_empty());
        // Surrounding whitespace does not turn one name into two.
        assert_eq!(
            parse_contract_command("contracts", "  analytics.public.orders  ")
                .unwrap()
                .unwrap(),
            ContractsCommand::Show {
                table: "analytics.public.orders".into(),
                profile: None,
            }
        );
    }

    /// `/contract` is kept as a silent alias of the merged command, not a
    /// second operation. Whatever the argument, it routes to the same command
    /// `/contracts` produces — so the two names can no longer disagree. (Kept as
    /// an alias rather than removed so the TUI completion registry — mirrored by
    /// `complete.rs` — stays in lockstep; see
    /// the SPEC REVIEW.)
    #[test]
    fn contract_is_a_silent_alias_of_the_merged_command() {
        // `/contract <table>` does what `/contracts <table>` does: Show.
        assert_eq!(
            parse_contract_command("contract", "analytics.public.orders")
                .unwrap()
                .unwrap(),
            parse_contract_command("contracts", "analytics.public.orders")
                .unwrap()
                .unwrap()
        );
        // `/contract` with no argument does what `/contracts` does: List. It is no
        // longer the "missing argument" error the old Show-only spelling raised.
        assert_eq!(
            parse_contract_command("contract", "").unwrap().unwrap(),
            parse_contract_command("contracts", "").unwrap().unwrap()
        );
        // Two tokens is a usage error on both spellings.
        assert!(parse_contract_command("contract", "a b").is_err());
        assert!(parse_contract_command("contracts", "a b").is_err());
    }

    #[test]
    fn parse_remember_table_kind_value() {
        let spec = parse_remember("a.b.c alias customers").unwrap();
        assert_eq!(spec.table, "a.b.c");
        assert_eq!(spec.kind, ClaimKindArg::Alias);
        assert_eq!(spec.value, "customers");
        assert_eq!(spec.column, None);
        assert_eq!(spec.reason, None);
    }

    #[test]
    fn parse_remember_directive_kind_carries_a_because_reason() {
        // The motivating case: a time-column claim with the reason a user would
        // state in one breath.
        let spec = parse_remember(
            "pagila.public.rental time-column return_date because a rental only counts once it comes back",
        )
        .unwrap();
        assert_eq!(spec.kind, ClaimKindArg::TimeColumn);
        assert_eq!(spec.value, "return_date");
        assert_eq!(
            spec.reason.as_deref(),
            Some("a rental only counts once it comes back")
        );
        // A grain with a reason.
        let grain =
            parse_remember("a.b.c grain one row per order because orders ship separately").unwrap();
        assert_eq!(grain.value, "one row per order");
        assert_eq!(grain.reason.as_deref(), Some("orders ship separately"));
        // A column-role with a reason.
        let role =
            parse_remember("a.b.c column-role amount measure because money the customer paid")
                .unwrap();
        assert_eq!(role.value, "measure");
        assert_eq!(role.reason.as_deref(), Some("money the customer paid"));
    }

    #[test]
    fn parse_remember_because_is_not_split_for_non_directive_kinds() {
        // A description legitimately contains "because"; the directive
        // constructors are the only ones that accept a reason, so for a
        // description the whole tail stays the value and no reason is split.
        let spec =
            parse_remember("a.b.c description returns because the warehouse closes").unwrap();
        assert_eq!(spec.kind, ClaimKindArg::Description);
        assert_eq!(spec.value, "returns because the warehouse closes");
        assert_eq!(spec.reason, None);
    }

    #[test]
    fn parse_remember_because_substring_is_not_a_split() {
        // "because" as a substring of a larger word is not a delimiter.
        let spec = parse_remember("a.b.c time-column created_at probecause_marker").unwrap();
        assert_eq!(spec.value, "created_at probecause_marker");
        assert_eq!(spec.reason, None);
        // A trailing `because` with no reason clause is not a split either.
        let bare = parse_remember("a.b.c time-column created_at because").unwrap();
        assert_eq!(bare.value, "created_at because");
        assert_eq!(bare.reason, None);
    }

    #[test]
    fn parse_remember_directive_without_because_has_no_reason() {
        let spec = parse_remember("a.b.c time-column created_at").unwrap();
        assert_eq!(spec.value, "created_at");
        assert_eq!(spec.reason, None);
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

    /// `/approve-all` defaults to the deny-by-default preview (no `--yes`),
    /// `--yes` flips it, an optional number overrides the limit, and anything
    /// else is a payload-free usage error.
    #[test]
    fn parse_approve_all_translates_to_the_batch_command() {
        assert_eq!(
            parse_contract_command("approve-all", "").unwrap().unwrap(),
            ContractsCommand::ApproveAll {
                profile: None,
                limit: None,
                yes: false,
            }
        );
        assert_eq!(
            parse_contract_command("approve-all", "--yes")
                .unwrap()
                .unwrap(),
            ContractsCommand::ApproveAll {
                profile: None,
                limit: None,
                yes: true,
            }
        );
        assert_eq!(
            parse_contract_command("approve-all", "--yes 10")
                .unwrap()
                .unwrap(),
            ContractsCommand::ApproveAll {
                profile: None,
                limit: Some(10),
                yes: true,
            }
        );
        // A token that is neither `--yes` nor a number is a usage error that
        // does not echo it.
        let bad = parse_contract_command("approve-all", "kaboom").unwrap_err();
        assert!(!bad.0.contains("kaboom"));
        assert!(!bad.0.is_empty());
        // Two limits or two --yes flags are usage errors, not silent overrides.
        assert!(parse_contract_command("approve-all", "10 20").is_err());
        assert!(parse_contract_command("approve-all", "--yes --yes").is_err());
    }
}
