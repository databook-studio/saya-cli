//! The database questions (S16 invariant 2c): engine, its required fields (a
//! password only ever as an environment-variable name), and the profile name.
//! DuckDB's read-only flag is asked because the connector refuses to guess it.

use saya_types::{DatabaseProfile, SecretRef};

use super::draft::{ProfileDraft, validate_env_name, validate_profile_name};
use super::prompt::{Cancel, Prompter, checked, text_field};

const DATABASE_CHOICES: [&str; 5] = ["sqlite", "duckdb", "postgresql", "mysql", "skip"];

/// A password env reference (blank skips; the value is never asked for).
fn env_ref(line: &str) -> Result<Option<SecretRef>, String> {
    match line.trim() {
        "" => Ok(None),
        trimmed => checked(trimmed, validate_env_name).map(|env| Some(SecretRef::Env { env })),
    }
}

fn port_with(default: u16) -> impl Fn(&str) -> Result<u16, String> {
    move |line| match line.trim() {
        "" => Ok(default),
        trimmed => trimmed
            .parse::<u16>()
            .map_err(|_| format!("{trimmed:?} is not a port number")),
    }
}

pub(crate) fn collect_database(
    prompter: &mut Prompter<'_>,
) -> Result<Option<ProfileDraft>, Cancel> {
    let choice = prompter.menu("Choose a database:", &DATABASE_CHOICES, Some(0))?;
    if choice == DATABASE_CHOICES.len() - 1 {
        return Ok(None); // skip
    }
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
            let suggested = if postgres {
                "SAYA_PG_PASSWORD"
            } else {
                "SAYA_MYSQL_PASSWORD"
            };
            let host = prompter.ask("Host: ", |l| text_field(l, None))?;
            let port =
                prompter.ask(&format!("Port [{default_port}]: "), port_with(default_port))?;
            let database = prompter.ask("Database name: ", |l| text_field(l, None))?;
            let user = prompter.ask("User: ", |l| text_field(l, None))?;
            let prompt = format!(
                "Password environment-variable name (suggested {suggested}; blank to skip): "
            );
            let password = prompter.ask(&prompt, env_ref)?;
            if postgres {
                DatabaseProfile::Postgres {
                    host,
                    port: Some(port),
                    database,
                    user,
                    ssl_mode: None,
                    password,
                }
            } else {
                DatabaseProfile::Mysql {
                    host,
                    port: Some(port),
                    database,
                    user,
                    ssl_mode: None,
                    ssl_ca: None,
                    password,
                }
            }
        }
        _ => unreachable!("the menu only offers the four engines and skip"),
    };
    let name = prompter.ask(&format!("Profile name [{engine}]: "), |line| {
        let trimmed = line.trim();
        let name = if trimmed.is_empty() { engine } else { trimmed };
        validate_profile_name(name)
            .map(|_| name.to_owned())
            .map_err(|error| error.to_string())
    })?;
    Ok(Some(ProfileDraft { name, profile }))
}
