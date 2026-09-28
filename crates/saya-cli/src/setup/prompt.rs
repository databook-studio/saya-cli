//! The setup prompts: a bounded [`Prompter`] over any reader/writer, plus the
//! question sets that turn answers into drafts. Every prompt is bounded
//! (three invalid answers, then cancel); EOF or a lost terminal cancels the
//! section ([`Cancel`]); no question asks for a secret value.
use std::io::{BufRead, Write};

use saya_config::AiProvider;
use saya_types::{DatabaseProfile, SecretRef};

use super::SetupError;
use super::draft::{ProfileDraft, ProviderDraft, validate_env_name, validate_profile_name};

/// Invalid answers one prompt tolerates before the section cancels (D11).
pub(crate) const MAX_INVALID_ANSWERS: usize = 3;

/// The section cancel carrier: EOF, burned tries, or a lost terminal.
pub(crate) struct Cancel;

pub(crate) type Step<T> = Result<T, Cancel>;

/// A line-oriented prompter over injected reader/writer (tests drive it).
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

    /// Asks until `read` accepts, at most [`MAX_INVALID_ANSWERS`] times; EOF,
    /// exhausted tries, or a lost terminal cancel the section.
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
    #[rustfmt::skip]
    pub(crate) fn confirm(&mut self, prompt: &str, default: bool) -> Step<bool> {
        self.ask(prompt, |line| {
            let trimmed = line.trim();
            Ok(if trimmed.is_empty() { default } else {
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

/// A text answer: blank takes the default when offered, else it is required.
#[rustfmt::skip]
fn text_field(line: &str, default: Option<&str>) -> Result<String, String> {
    match (line.trim(), default) {
        ("", Some(default)) => Ok(default.to_owned()),
        ("", None) => Err("this field is required".to_owned()),
        (trimmed, _) => Ok(trimmed.to_owned()),
    }
}

/// A name checked against a draft validator (env var, profile).
#[rustfmt::skip]
fn checked(name: &str, validate: fn(&str) -> Result<(), SetupError>) -> Result<String, String> {
    validate(name).map(|_| name.to_owned()).map_err(|error| error.to_string())
}

/// An env-var name: blank takes `suggested` only when `accept_suggested`,
/// otherwise blank skips; the value is never asked for.
#[rustfmt::skip]
fn env_field(line: &str, suggested: Option<&str>, accept_suggested: bool) -> Result<Option<String>, String> {
    match (line.trim(), suggested, accept_suggested) {
        ("", Some(suggested), true) => Ok(Some(suggested.to_owned())),
        ("", _, _) => Ok(None),
        (trimmed, _, _) => checked(trimmed, validate_env_name).map(Some),
    }
}

/// A password env reference (blank skips; value never asked for).
#[rustfmt::skip]
fn env_ref(line: &str) -> Result<Option<SecretRef>, String> {
    match line.trim() {
        "" => Ok(None),
        trimmed => checked(trimmed, validate_env_name).map(|env| Some(SecretRef::Env { env })),
    }
}

#[rustfmt::skip]
fn port_with(default: u16) -> impl Fn(&str) -> Result<u16, String> {
    move |line| match line.trim() {
        "" => Ok(default),
        trimmed => trimmed.parse::<u16>().map_err(|_| format!("{trimmed:?} is not a port number")),
    }
}

/// What a provider asks beyond its model: ollama is local (default base URL,
/// no key); cloud providers carry a suggested key variable; an
/// openai_compatible endpoint must state its base URL.
#[derive(Clone, Copy)]
#[rustfmt::skip]
enum ProviderAsk { Local(&'static str), Cloud(&'static str), Compatible }

#[rustfmt::skip]
const PROVIDER_CHOICES: [(AiProvider, &str, ProviderAsk); 5] = [
    (AiProvider::Ollama,           "qwen2.5-coder:14b", ProviderAsk::Local("http://localhost:11434")),
    (AiProvider::Openai,           "gpt-4o-mini",       ProviderAsk::Cloud("OPENAI_API_KEY")),
    (AiProvider::OpenaiCompatible, "gpt-4o-mini",       ProviderAsk::Compatible),
    (AiProvider::Anthropic,        "claude-sonnet-4-5", ProviderAsk::Cloud("ANTHROPIC_API_KEY")),
    (AiProvider::Gemini,           "gemini-2.5-flash",  ProviderAsk::Cloud("GEMINI_API_KEY")),
];

/// The provider questions: model (default shown, editable), base URL where
/// needed, and the key's environment-variable NAME — the value is never
/// asked for, echoed, or stored; only its presence is reported.
#[rustfmt::skip]
pub(crate) fn collect_provider(prompter: &mut Prompter<'_>) -> Step<Option<ProviderDraft>> {
    let mut names: Vec<&str> = PROVIDER_CHOICES.iter().map(|(p, ..)| p.as_str()).collect();
    names.push("skip");
    let choice = prompter.menu("Choose an AI provider:", &names, Some(0))?;
    if choice == PROVIDER_CHOICES.len() { return Ok(None); }
    let (provider, def, ask) = PROVIDER_CHOICES[choice];
    let model = prompter.ask(&format!("Model [{def}]: "), |l| text_field(l, Some(def)))?;
    let (base_url, api_key_env) = match ask {
        ProviderAsk::Local(url) =>
            (Some(prompter.ask(&format!("Base URL [{url}]: "), |l| text_field(l, Some(url)))?), None),
        ProviderAsk::Cloud(suggested) => {
            let prompt = format!("API key environment variable name [{suggested}] (never the key itself): ");
            (None, prompter.ask(&prompt, |l| env_field(l, Some(suggested), true))?)
        }
        ProviderAsk::Compatible => {
            let url = prompter.ask("Base URL: ", |l| text_field(l, None))?;
            let prompt = "API key environment variable name (blank to skip; never the key itself): ";
            (Some(url), prompter.ask(prompt, |l| env_field(l, None, false))?)
        }
    };
    if let Some(env) = &api_key_env {
        let state = if std::env::var_os(env).is_some() { "is currently set" } else { "is not set" };
        prompter.say(&format!("{env} {state}.")).map_err(|_| Cancel)?;
    }
    Ok(Some(ProviderDraft { provider, model, base_url, api_key_env }))
}

const DATABASE_CHOICES: [&str; 5] = ["sqlite", "duckdb", "postgresql", "mysql", "skip"];

/// The database questions: engine, its required fields (a password only as
/// an env-var name), and the profile name. DuckDB's read-only flag is asked
/// because the connector refuses to guess it.
#[rustfmt::skip]
pub(crate) fn collect_database(prompter: &mut Prompter<'_>) -> Step<Option<ProfileDraft>> {
    let choice = prompter.menu("Choose a database:", &DATABASE_CHOICES, Some(0))?;
    if choice == DATABASE_CHOICES.len() - 1 { return Ok(None); }
    let engine = DATABASE_CHOICES[choice];
    let profile = match engine {
        "sqlite" => DatabaseProfile::Sqlite {
            path: prompter.ask("Database file path: ", |l| text_field(l, None))?,
            read_only: true,
        },
        "duckdb" => DatabaseProfile::DuckDb {
            path: prompter.ask("Database file path: ", |l| text_field(l, None))?,
            read_only: Some(prompter.confirm("Open the file read-only? [Y/n] ", true)?),
        },
        "postgresql" | "mysql" => {
            let postgres = engine == "postgresql";
            let default_port: u16 = if postgres { 5432 } else { 3306 };
            let suggested = if postgres { "SAYA_PG_PASSWORD" } else { "SAYA_MYSQL_PASSWORD" };
            let host = prompter.ask("Host: ", |l| text_field(l, None))?;
            let port = prompter.ask(&format!("Port [{default_port}]: "), port_with(default_port))?;
            let database = prompter.ask("Database name: ", |l| text_field(l, None))?;
            let user = prompter.ask("User: ", |l| text_field(l, None))?;
            let prompt = format!("Password environment-variable name (suggested {suggested}; blank to skip): ");
            let password = prompter.ask(&prompt, env_ref)?;
            if postgres {
                DatabaseProfile::Postgres { host, port: Some(port), database, user, ssl_mode: None, password }
            } else {
                DatabaseProfile::Mysql { host, port: Some(port), database, user, ssl_mode: None, ssl_ca: None, password }
            }
        }
        _ => unreachable!("the menu only offers the four engines and skip"),
    };
    let name = prompter.ask(&format!("Profile name [{engine}]: "), |l| {
        let trimmed = l.trim();
        checked(if trimmed.is_empty() { engine } else { trimmed }, validate_profile_name)
    })?;
    Ok(Some(ProfileDraft { name, profile }))
}
