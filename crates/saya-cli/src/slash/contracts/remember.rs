//! The `/remember` argument parser: turns the tail after `/remember` into a
//! [`RememberSpec`] the dispatcher forwards to `ContractsCommand::Remember`.
//!
//! The table and kind are positional; for table-scoped kinds everything after
//! the kind is the value (so a description with spaces is natural), and for
//! column-scoped kinds the column is the next positional and everything after
//! it is the value. A trailing `because <reason…>` clause is split off only for
//! the directive kinds (grain, time-column, column-role); for description and
//! alias it stays on the value.

use crate::cli::ClaimKindArg;
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

#[cfg(test)]
#[path = "remember_tests.rs"]
mod tests;
