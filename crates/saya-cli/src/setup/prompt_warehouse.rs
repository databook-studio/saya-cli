//! The warehouse question sets: ClickHouse and BigQuery, plus the dispatch
//! for Snowflake (`prompt_snowflake`). Only each variant's required fields
//! are demanded; optional fields show their default and blank skips them.
//! Every secret is taken as an environment-variable name or a file path —
//! never a value.

use saya_types::{DatabaseProfile, SecretRef};

use super::prompt::{Cancel, Prompter, text_field};
use super::prompt_database::env_ref;
use super::prompt_snowflake;

/// Collects one warehouse profile; `engine` is the menu choice string.
pub(crate) fn collect(
    prompter: &mut Prompter<'_>,
    engine: &str,
) -> Result<DatabaseProfile, Cancel> {
    match engine {
        "snowflake" => prompt_snowflake::snowflake(prompter),
        "clickhouse" => clickhouse(prompter),
        "bigquery" => bigquery(prompter),
        _ => unreachable!("the menu only offers the warehouse engines here"),
    }
}

/// Blank skips an optional field.
pub(crate) fn optional(line: &str) -> Result<Option<String>, String> {
    let trimmed = line.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

/// Blank takes the connector's default for an optional numeric field.
pub(crate) fn optional_parse<T: std::str::FromStr>(
    what: &str,
) -> impl Fn(&str) -> Result<Option<T>, String> {
    move |line| match line.trim() {
        "" => Ok(None),
        trimmed => trimmed
            .parse::<T>()
            .map(Some)
            .map_err(|_| format!("{trimmed:?} is not {what}")),
    }
}

/// A file path for a key stored on disk; the path is the only thing asked for.
pub(crate) fn required_file(line: &str) -> Result<SecretRef, String> {
    text_field(line, None).map(|file| SecretRef::File { file })
}

fn clickhouse(prompter: &mut Prompter<'_>) -> Result<DatabaseProfile, Cancel> {
    let host = prompter.ask("ClickHouse host: ", |l| text_field(l, None))?;
    let secure = prompter.confirm("Use TLS (secure = true)? [y/N] ", false)?;
    let default_port = if secure { 8443 } else { 8123 };
    let port = prompter.ask(
        &format!("Port [{default_port}]: "),
        optional_parse::<u16>("a port number"),
    )?;
    let database = prompter.ask("Database name (blank to skip): ", optional)?;
    let user = prompter.ask("User [default]: ", optional)?;
    let password = prompter.ask(
        "Password environment-variable name (blank to skip): ",
        env_ref,
    )?;
    Ok(DatabaseProfile::ClickHouse {
        host,
        port,
        database,
        user,
        password,
        secure: Some(secure),
    })
}

fn bigquery(prompter: &mut Prompter<'_>) -> Result<DatabaseProfile, Cancel> {
    let project = prompter.ask("GCP project id: ", |l| text_field(l, None))?;
    let dataset = prompter.ask("Default dataset (blank to skip): ", optional)?;
    let location = prompter.ask("Job location, e.g. US or EU (blank to skip): ", optional)?;
    let max_bytes_billed = prompter.ask(
        "Max bytes billed per query (blank for the 1 TiB default): ",
        optional_parse::<u64>("a number"),
    )?;
    let service_account_key =
        prompter.ask("Service-account JSON key file path: ", required_file)?;
    Ok(DatabaseProfile::BigQuery {
        project,
        dataset,
        location,
        max_bytes_billed,
        service_account_key,
    })
}
