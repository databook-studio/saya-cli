//! Renders the TOML snippets a plan writes: the `[ai]` section and one
//! `[profiles.<name>]` block. Scalars go through the real TOML serializer so
//! escaping is never hand-written.

use std::collections::BTreeMap;

use saya_types::DatabaseProfile;
use serde::Serialize;

use super::{SetupError, draft::ProviderDraft};

#[derive(Serialize)]
struct AiHead<'a> {
    provider: &'a str,
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<&'a str>,
}

#[derive(Serialize)]
struct AiSection<'a> {
    ai: AiHead<'a>,
}

/// The `[ai]` section for a provider draft, with every scalar TOML-escaped by
/// the serializer. The `api_key` line is composed because a nested table would
/// otherwise render as an `[ai.api_key]` header; the env name is validated to
/// `[A-Z_][A-Z0-9_]{0,63}`, so the interpolation cannot need escaping.
pub(crate) fn ai_section(provider: &ProviderDraft) -> Result<String, SetupError> {
    let mut section = toml::to_string(&AiSection {
        ai: AiHead {
            provider: provider.provider.as_str(),
            model: &provider.model,
            base_url: provider.base_url.as_deref(),
        },
    })
    .map_err(|error| SetupError::Render(error.to_string()))?;
    if let Some(env) = &provider.api_key_env {
        section.push_str(&format!("api_key = {{ env = \"{env}\" }}\n"));
    }
    Ok(section)
}

#[derive(Serialize)]
struct ProfilesFile<'a> {
    profiles: BTreeMap<&'a str, &'a DatabaseProfile>,
}

/// The `[profiles.<name>]` block for one draft profile, via TOML
/// serialization of the `{ profiles: { <name>: profile } }` wrapper — names
/// with dots or dashes come out quoted and round-trip.
pub(crate) fn profile_block(name: &str, profile: &DatabaseProfile) -> Result<String, SetupError> {
    if !matches!(
        profile,
        DatabaseProfile::Postgres { .. }
            | DatabaseProfile::Mysql { .. }
            | DatabaseProfile::Sqlite { .. }
            | DatabaseProfile::DuckDb { .. }
    ) {
        return Err(SetupError::UnsupportedEngine(format!(
            "configure {} in connections.toml; see docs/connections.md",
            engine_name(profile)
        )));
    }
    let mut profiles = BTreeMap::new();
    profiles.insert(name, profile);
    toml::to_string(&ProfilesFile { profiles })
        .map_err(|error| SetupError::Render(error.to_string()))
}

fn engine_name(profile: &DatabaseProfile) -> &'static str {
    match profile {
        DatabaseProfile::Postgres { .. } => "postgresql",
        DatabaseProfile::Mysql { .. } => "mysql",
        DatabaseProfile::DuckDb { .. } => "duckdb",
        DatabaseProfile::Sqlite { .. } => "sqlite",
        DatabaseProfile::Snowflake { .. } => "snowflake",
        DatabaseProfile::ClickHouse { .. } => "clickhouse",
        DatabaseProfile::BigQuery { .. } => "bigquery",
    }
}
