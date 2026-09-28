//! The setup prompts: a bounded [`Prompter`] over any reader/writer, plus the
//! shared answer parsers. Every prompt is bounded (three invalid answers,
//! then cancel); EOF or a lost terminal cancels the section ([`Cancel`]); no
//! question asks for a secret value.

use std::io::{BufRead, Write};

use super::SetupError;
use super::draft::validate_env_name;

/// Invalid answers one prompt tolerates before the section cancels (D11).
pub(crate) const MAX_INVALID_ANSWERS: usize = 3;

/// The section cancel carrier: EOF, burned tries, or a lost terminal.
pub(crate) struct Cancel;

pub(crate) type Step<T> = Result<T, Cancel>;

/// A line-oriented prompter over injected reader/writer, so tests drive
/// scripted input and capture every byte the flow shows (the
/// `session_trust::ask_trust` pattern).
pub(crate) struct Prompter<'a> {
    input: &'a mut dyn BufRead,
    output: &'a mut dyn Write,
}

impl<'a> Prompter<'a> {
    pub(crate) fn new(input: &'a mut dyn BufRead, output: &'a mut dyn Write) -> Self {
        Self { input, output }
    }

    /// Writes one line and flushes, so it is visible before the next read.
    pub(crate) fn say(&mut self, line: &str) -> std::io::Result<()> {
        self.write(&format!("{line}\n"))
    }

    fn write(&mut self, prompt: &str) -> std::io::Result<()> {
        self.output.write_all(prompt.as_bytes())?;
        self.output.flush()
    }

    /// Asks until `parse` accepts, at most [`MAX_INVALID_ANSWERS`] times;
    /// EOF, exhausted tries, or a lost terminal cancel the section.
    pub(crate) fn ask<T>(
        &mut self,
        prompt: &str,
        read: impl Fn(&str) -> Result<T, String>,
    ) -> Step<T> {
        for _ in 0..MAX_INVALID_ANSWERS {
            self.write(prompt).map_err(|_| Cancel)?;
            let mut line = String::new();
            let taken = self.input.read_line(&mut line).map_err(|_| Cancel)?;
            if taken == 0 {
                return Err(Cancel);
            }
            match read(&line) {
                Ok(value) => return Ok(value),
                Err(reason) => self.say(&reason).map_err(|_| Cancel)?,
            }
        }
        Err(Cancel)
    }

    /// Yes/no: blank takes `default`, y/yes is yes, anything else no, EOF
    /// cancels. Never retried, so the `[y/N]` hint is final.
    pub(crate) fn confirm(&mut self, prompt: &str, default: bool) -> Step<bool> {
        self.ask(prompt, |line| {
            let trimmed = line.trim();
            Ok(if trimmed.is_empty() {
                default
            } else {
                trimmed.eq_ignore_ascii_case("y") || trimmed.eq_ignore_ascii_case("yes")
            })
        })
    }

    /// A numbered menu: a 1-based number or an option name (case-insensitive);
    /// blank takes `default`; bounded like every prompt.
    pub(crate) fn menu(
        &mut self,
        title: &str,
        options: &[&str],
        default: Option<usize>,
    ) -> Step<usize> {
        let mut text = String::from(title);
        text.push('\n');
        for (index, option) in options.iter().enumerate() {
            text.push_str(&format!("  {}) {option}\n", index + 1));
        }
        if let Some(index) = default {
            text.push_str(&format!("Choose a number or name [{}]: ", options[index]));
        } else {
            text.push_str("Choose a number or name: ");
        }
        self.ask(&text, |line| {
            let answer = line.trim();
            if answer.is_empty() {
                return default.ok_or_else(|| "choose an option".to_owned());
            }
            let by_number = answer
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=options.len()).contains(n));
            let by_name = options
                .iter()
                .position(|option| option.eq_ignore_ascii_case(answer));
            by_number.map(|n| n - 1).or(by_name).ok_or_else(|| {
                format!(
                    "unrecognized answer {answer:?}; pick 1-{} or a name",
                    options.len()
                )
            })
        })
    }
}

// -- answer parsers shared by the question sets --

/// A text answer: blank takes the default when offered, else it is required.
pub(crate) fn text_field(line: &str, default: Option<&str>) -> Result<String, String> {
    match (line.trim(), default) {
        ("", Some(default)) => Ok(default.to_owned()),
        ("", None) => Err("this field is required".to_owned()),
        (trimmed, _) => Ok(trimmed.to_owned()),
    }
}

/// A name checked against a draft validator (env var, profile).
pub(crate) fn checked(
    name: &str,
    validate: fn(&str) -> Result<(), SetupError>,
) -> Result<String, String> {
    validate(name)
        .map(|_| name.to_owned())
        .map_err(|error| error.to_string())
}

/// An env-var name: blank takes `suggested` only when `accept_suggested`,
/// otherwise blank skips; the value is never asked for.
pub(crate) fn env_field(
    line: &str,
    suggested: Option<&str>,
    accept_suggested: bool,
) -> Result<Option<String>, String> {
    match (line.trim(), suggested, accept_suggested) {
        ("", Some(suggested), true) => Ok(Some(suggested.to_owned())),
        ("", _, _) => Ok(None),
        (trimmed, _, _) => checked(trimmed, validate_env_name).map(Some),
    }
}
