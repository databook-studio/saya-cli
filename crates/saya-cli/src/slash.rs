use saya_agent::ApprovalPolicy;
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Connect(String),
    Connections,
    Include(String),
    Exclude(String),
    Provider(Option<String>),
    Model(Option<String>),
    Privacy(Option<bool>),
    Approvals(Option<ApprovalPolicy>),
    Schema(bool),
    Clear,
    History,
    Help,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashParseError(pub String);

impl fmt::Display for SlashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SlashParseError {}

pub fn parse_slash_command(input: &str) -> Result<Option<SlashCommand>, SlashParseError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }
    let mut parts = trimmed[1..].split_whitespace();
    let name = parts.next().unwrap_or_default();
    let arg = parts.collect::<Vec<_>>().join(" ");
    let required = || {
        (!arg.is_empty())
            .then_some(arg.clone())
            .ok_or_else(|| SlashParseError("command requires an argument".into()))
    };
    let command = match name {
        "connect" => SlashCommand::Connect(required()?),
        "connections" => SlashCommand::Connections,
        "include" => SlashCommand::Include(required()?),
        "exclude" => SlashCommand::Exclude(required()?),
        "provider" => SlashCommand::Provider((!arg.is_empty()).then_some(arg)),
        "model" => SlashCommand::Model((!arg.is_empty()).then_some(arg)),
        "privacy" => SlashCommand::Privacy(parse_bool(&arg)?),
        "approvals" => SlashCommand::Approvals(parse_approval(&arg)?),
        "schema" => SlashCommand::Schema(arg == "refresh"),
        "clear" => SlashCommand::Clear,
        "history" => SlashCommand::History,
        "help" => SlashCommand::Help,
        "exit" | "quit" => SlashCommand::Exit,
        other => return Err(SlashParseError(format!("unknown command: /{other}"))),
    };
    Ok(Some(command))
}

fn parse_bool(value: &str) -> Result<Option<bool>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    match value {
        "on" | "true" | "enable" => Ok(Some(true)),
        "off" | "false" | "disable" => Ok(Some(false)),
        _ => Err(SlashParseError("privacy expects on or off".into())),
    }
}

fn parse_approval(value: &str) -> Result<Option<ApprovalPolicy>, SlashParseError> {
    if value.is_empty() {
        return Ok(None);
    }
    ApprovalPolicy::from_str(value)
        .map(Some)
        .map_err(|error| SlashParseError(error.to_string()))
}

pub fn help_text() -> &'static str {
    "/connect <profile>  /connections  /include <profile>  /exclude <profile>\n/provider [name]     /model [name]  /privacy [on|off]\n/approvals [ask|read-only|never]  /schema [refresh]  /clear\n/history  /help  /exit"
}
