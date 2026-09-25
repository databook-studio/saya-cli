mod arguments;
mod commands;

#[cfg(test)]
use commands::description_for;

#[cfg(test)]
#[path = "complete_tests.rs"]
mod tests;

/// A single completion candidate for the slash popup.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) value: String,
    pub(crate) description: Option<String>,
}

/// Given the current input line, returns the candidates for the slash popup plus
/// the half-open CHAR range [start, end) in `line` that accepting a candidate
/// replaces. Returns None when the popup should not be shown (line does not
/// start with '/', or the command takes no completable argument).
#[allow(dead_code)]
pub(crate) fn slash_candidates(
    line: &str,
    profiles: &[String],
) -> Option<(usize, usize, Vec<Candidate>)> {
    if !line.starts_with('/') {
        return None;
    }

    let total_chars = line.chars().count();

    if let Some(byte_idx) = line.rfind(' ') {
        let cmd_word = line[1..]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_lowercase();

        let arg = &line[byte_idx + 1..];
        let start_char = line[..byte_idx].chars().count() + 1;
        arguments::argument_candidates(&cmd_word, arg, profiles, start_char, total_chars)
    } else {
        let prefix = &line[1..];
        commands::command_candidates(prefix, total_chars)
    }
}
