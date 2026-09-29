//! The flat-tail flag grammar the `/investigation` parsers share: token
//! spans over a raw line, the positional/value/boolean scan, and the
//! value/id helpers. The tail is a flat line, not argv, so a value flag's
//! value runs to the next known flag or the tail's end, verbatim — SQL may
//! carry `--` comments, so unknown `--`-shaped tokens are usage errors only
//! where a name or id belongs.

use crate::slash::SlashParseError;

/// Whitespace-delimited token spans (start, end) of `tail`, so the parsers
/// keep the text between tokens verbatim.
pub(super) fn token_spans(tail: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < tail.len() {
        let rest = &tail[index..];
        index += rest.len() - rest.trim_start().len();
        if index >= tail.len() {
            break;
        }
        let end = tail[index..]
            .find(char::is_whitespace)
            .map_or(tail.len(), |offset| index + offset);
        spans.push((index, end));
        index = end;
    }
    spans
}

/// The parsed flag grammar: the positional zone's text (before the first
/// known flag), each value flag's captured text in order, the boolean
/// flags seen, and the repeatable flags' captured texts in order.
pub(super) struct Scan {
    pub(super) positional: String,
    pub(super) values: Vec<(&'static str, String)>,
    pub(super) booleans: Vec<&'static str>,
    pub(super) repeats: Vec<(&'static str, String)>,
}

/// Scans `tail` as a positional zone followed by `--value <value…>`,
/// `--boolean`, and repeatable `--flag <value…>` flags in any order. A value
/// runs to the next known flag or the tail's end; unknown `--`-shaped tokens
/// inside a value zone are value text (SQL comments), anywhere else they are
/// usage errors — never swallowed names or ids. A repeatable flag may be
/// given any number of times; the others refuse a second occurrence.
pub(super) fn scan(
    tail: &str,
    value_flags: &[&'static str],
    boolean_flags: &[&'static str],
    repeat_flags: &[&'static str],
    usage: &str,
) -> Result<Scan, SlashParseError> {
    let known = |token: &str| {
        value_flags.contains(&token)
            || boolean_flags.contains(&token)
            || repeat_flags.contains(&token)
    };
    let spans = token_spans(tail);
    let zone_start = spans
        .iter()
        .find(|(start, end)| known(&tail[*start..*end]))
        .map(|(start, _)| *start)
        .unwrap_or(tail.len());
    // A flag-shaped token where the positional belongs is a usage error.
    for &(start, end) in spans
        .iter()
        .take(spans.partition_point(|(s, _)| *s < zone_start))
    {
        let token = &tail[start..end];
        if token.starts_with("--") {
            return Err(unknown_flag_error("investigation flag", token, usage));
        }
    }
    let mut values = Vec::new();
    let mut booleans = Vec::new();
    let mut repeats = Vec::new();
    // The open value flag and its value's byte start, while its zone runs.
    let mut open: Option<(&'static str, usize)> = None;
    for &(start, end) in &spans[spans.partition_point(|(s, _)| *s < zone_start)..] {
        let token = &tail[start..end];
        let flag = boolean_flags
            .iter()
            .chain(value_flags.iter())
            .chain(repeat_flags.iter())
            .find(|candidate| **candidate == token)
            .copied();
        // A known flag closes any open value; a value flag opens a new one.
        if let Some(flag) = flag {
            close_zone(
                tail,
                open.take(),
                start,
                repeat_flags,
                &mut values,
                &mut repeats,
                usage,
            )?;
            if boolean_flags.contains(&token) {
                if booleans.contains(&flag) {
                    return Err(SlashParseError(format!("{token} given twice{usage}")));
                }
                booleans.push(flag);
            } else if repeat_flags.contains(&token) {
                open = Some((flag, end));
            } else if values.iter().any(|(given, _)| *given == flag) {
                return Err(SlashParseError(format!("{token} given twice{usage}")));
            } else {
                open = Some((flag, end));
            }
        } else if open.is_none() {
            let error = if token.starts_with("--") {
                unknown_flag_error("investigation flag", token, usage)
            } else {
                SlashParseError(format!("unexpected argument{usage}"))
            };
            return Err(error);
        }
    }
    close_zone(
        tail,
        open.take(),
        tail.len(),
        repeat_flags,
        &mut values,
        &mut repeats,
        usage,
    )?;
    Ok(Scan {
        positional: tail[..zone_start].trim().to_string(),
        values,
        booleans,
        repeats,
    })
}

/// Closes the open value zone into `values` or `repeats`, by flag kind: a
/// repeatable flag accumulates every occurrence, the others refuse a second.
fn close_zone(
    tail: &str,
    open: Option<(&'static str, usize)>,
    end: usize,
    repeat_flags: &[&'static str],
    values: &mut Vec<(&'static str, String)>,
    repeats: &mut Vec<(&'static str, String)>,
    usage: &str,
) -> Result<(), SlashParseError> {
    let Some((open_flag, value_start)) = open else {
        return Ok(());
    };
    let value = tail[value_start..end].trim();
    if value.is_empty() {
        return Err(SlashParseError(format!("{open_flag} needs a value{usage}")));
    }
    if repeat_flags.contains(&open_flag) {
        repeats.push((open_flag, value.to_string()));
    } else {
        values.push((open_flag, value.to_string()));
    }
    Ok(())
}

/// Removes one flag's value from the scan, if it was given.
pub(super) fn take_value(values: &mut Vec<(&'static str, String)>, flag: &str) -> Option<String> {
    let index = values
        .iter()
        .position(|(candidate, _)| *candidate == flag)?;
    Some(values.remove(index).1)
}

/// Removes one repeatable flag's captured values, in the order given.
pub(super) fn take_values(repeats: &mut Vec<(&'static str, String)>, flag: &str) -> Vec<String> {
    let mut taken = Vec::new();
    let mut kept = Vec::new();
    for (candidate, value) in repeats.drain(..) {
        if candidate == flag {
            taken.push(value);
        } else {
            kept.push((candidate, value));
        }
    }
    *repeats = kept;
    taken
}

/// A number-valued flag: the value must parse, or it is a usage error.
pub(super) fn take_number<T: std::str::FromStr>(
    values: &mut Vec<(&'static str, String)>,
    flag: &str,
    usage: &str,
) -> Result<Option<T>, SlashParseError> {
    match take_value(values, flag) {
        Some(value) => value
            .parse::<T>()
            .map(Some)
            .map_err(|_| SlashParseError(format!("{flag} needs a number{usage}"))),
        None => Ok(None),
    }
}

/// The positional id: exactly one whitespace-free token.
pub(super) fn id_from(positional: &str, usage: &str) -> Result<String, SlashParseError> {
    if positional.split_whitespace().count() != 1 {
        return Err(SlashParseError(format!("expected a single id{usage}")));
    }
    Ok(positional.to_string())
}

/// The usage error for an unknown flag-shaped token (D2/D8): the token is
/// echoed only up to (not including) its first `=` — a refused token's value
/// must never reach the transcript or the saved session — and the attached
/// `--param=` spelling adds the spelling hint. A token without `=` echoes
/// whole, as before. `kind` names the refused slot ("investigation flag",
/// "investigation subcommand").
pub(super) fn unknown_flag_error(kind: &str, token: &str, usage: &str) -> SlashParseError {
    let echo = match token.split_once('=') {
        Some((head, _)) => format!("{head}=…"),
        None => token.to_owned(),
    };
    let hint = if token.starts_with("--param=") {
        "; use --param name=value"
    } else {
        ""
    };
    SlashParseError(format!("unknown {kind}: {echo}{usage}{hint}"))
}

#[cfg(test)]
#[path = "flags_tests.rs"]
mod tests;
