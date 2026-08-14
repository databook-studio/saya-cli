//! Slash-text → `PreferencesCommand` translation for the `/preferences` slash
//! adapter. This is the only new slash surface in the 5c-2 slice: it turns
//! `/preferences` into the same `PreferencesCommand::List` the headless `saya
//! preferences list` clap parser produces, so both paths hand the same typed
//! value to [`crate::commands::run_preferences`]. No second scope resolution,
//! no second DTO mapping lives here — those stay inside the shared dispatcher.
//! The parity tests in `tests/preferences_slash_parity.rs` assert the
//! translated command equals the headless one.

use crate::cli::PreferencesCommand;
use crate::slash::SlashParseError;

/// Translates a `/preferences` argument tail into the matching
/// `PreferencesCommand`, or a usage error. `/preferences` mirrors `list` only —
/// `set`/`unset` stay headless — so it takes no argument and always resolves to
/// `List { profile: None }` (the slash path uses the active profile, matching
/// `/contracts` → `List`).
pub(crate) fn parse_preferences_command(
    name: &str,
    arg: &str,
) -> Result<Option<PreferencesCommand>, SlashParseError> {
    if name != "preferences" {
        return Ok(None);
    }
    if !arg.trim().is_empty() {
        return Err(SlashParseError("/preferences takes no argument".into()));
    }
    Ok(Some(PreferencesCommand::List { profile: None }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_preferences_no_arg() {
        let cmd = parse_preferences_command("preferences", "")
            .unwrap()
            .unwrap();
        assert_eq!(cmd, PreferencesCommand::List { profile: None });
    }

    #[test]
    fn parse_preferences_rejects_arg() {
        assert!(parse_preferences_command("preferences", "x").is_err());
    }

    #[test]
    fn parse_other_name_returns_none() {
        assert!(
            parse_preferences_command("contracts", "")
                .unwrap()
                .is_none()
        );
    }
}
