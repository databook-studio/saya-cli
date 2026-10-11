use super::{SlashCommand, SlashParseError};
use clap::Parser;
#[derive(Parser)]
struct CodingArgs {
    #[command(subcommand)]
    command: crate::cli::CodingCommand,
}
pub(super) fn parse(tail: &str) -> Result<SlashCommand, SlashParseError> {
    if tail.len() > 32 * 1024 {
        return Err(SlashParseError(
            "coding command exceeds its byte bound".into(),
        ));
    }
    let words = crate::interactive::session::run::tokenize_tail(tail).map_err(SlashParseError)?;
    CodingArgs::try_parse_from(std::iter::once("coding".to_string()).chain(words))
        .map(|args| SlashCommand::Coding(args.command))
        .map_err(|error| SlashParseError(error.to_string()))
}
