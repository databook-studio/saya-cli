//! The `request_clarification` argument contract (B3c): the tool name, the
//! bounds, and the parse the loop and the definition share. One question, up
//! to six candidate answers — nothing else is accepted.

use thiserror::Error;

/// The tool name the model calls to stop and ask the user one focused
/// question instead of assuming. Recognised by the loop before the ordinary
/// tool path; the CLI advertises the matching definition under this exact
/// name.
pub const REQUEST_CLARIFICATION_TOOL: &str = "request_clarification";

/// The question's character bound.
pub const MAX_CLARIFICATION_QUESTION_CHARS: usize = 300;
/// How many options one ask may carry.
pub const MAX_CLARIFICATION_OPTIONS: usize = 6;
/// Each option's character bound.
pub const MAX_CLARIFICATION_OPTION_CHARS: usize = 80;

/// A sanitised, bounded clarification ask, parsed from a call's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clarification {
    pub question: String,
    pub options: Vec<String>,
}

/// Why a `request_clarification` call could not be honoured. The reason is
/// fed back as the tool result so the model can correct itself, and the turn
/// continues — a malformed ask never ends a turn.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClarificationError {
    #[error("invalid tool arguments: expected an object")]
    NotAnObject,
    #[error("invalid tool arguments: question must be a string")]
    QuestionNotString,
    #[error("invalid tool arguments: question must not be empty")]
    QuestionEmpty,
    #[error("invalid tool arguments: question is over the {limit}-character bound")]
    QuestionTooLong { limit: usize },
    #[error("invalid tool arguments: options must be an array of strings")]
    OptionsNotStringArray,
    #[error("invalid tool arguments: at most {limit} options are allowed")]
    TooManyOptions { limit: usize },
    #[error("invalid tool arguments: an option must not be empty")]
    OptionEmpty,
    #[error("invalid tool arguments: an option is over the {limit}-character bound")]
    OptionTooLong { limit: usize },
}

/// Parses and bounds one call's arguments. Control characters are stripped
/// (newlines and tabs kept) and whitespace is trimmed before the bounds are
/// checked, so the event the loop emits never carries either.
pub fn parse(arguments: &serde_json::Value) -> Result<Clarification, ClarificationError> {
    let object = arguments
        .as_object()
        .ok_or(ClarificationError::NotAnObject)?;
    let question = object
        .get("question")
        .and_then(serde_json::Value::as_str)
        .ok_or(ClarificationError::QuestionNotString)?;
    let question = bound_question(&sanitize(question))?;
    let options = match object.get("options") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(value) => bound_options(value)?,
    };
    Ok(Clarification { question, options })
}

fn bound_question(sanitised: &str) -> Result<String, ClarificationError> {
    let question = sanitised.trim();
    if question.is_empty() {
        return Err(ClarificationError::QuestionEmpty);
    }
    if question.chars().count() > MAX_CLARIFICATION_QUESTION_CHARS {
        return Err(ClarificationError::QuestionTooLong {
            limit: MAX_CLARIFICATION_QUESTION_CHARS,
        });
    }
    Ok(question.to_owned())
}

fn bound_options(value: &serde_json::Value) -> Result<Vec<String>, ClarificationError> {
    let list = value
        .as_array()
        .ok_or(ClarificationError::OptionsNotStringArray)?;
    if list.len() > MAX_CLARIFICATION_OPTIONS {
        return Err(ClarificationError::TooManyOptions {
            limit: MAX_CLARIFICATION_OPTIONS,
        });
    }
    list.iter()
        .map(|entry| {
            let option = entry
                .as_str()
                .ok_or(ClarificationError::OptionsNotStringArray)?;
            let option = sanitize(option).trim().to_owned();
            if option.is_empty() {
                return Err(ClarificationError::OptionEmpty);
            }
            if option.chars().count() > MAX_CLARIFICATION_OPTION_CHARS {
                return Err(ClarificationError::OptionTooLong {
                    limit: MAX_CLARIFICATION_OPTION_CHARS,
                });
            }
            Ok(option)
        })
        .collect()
}

/// Strips control characters, keeping newlines and tabs — the same classes
/// the terminal renderers strip, applied here at the source.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|c| matches!(c, '\n' | '\t') || !c.is_control())
        .collect()
}
