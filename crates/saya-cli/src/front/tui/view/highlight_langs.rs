//! The languages a fenced block can name, and the little each one needs to be
//! read as code: its comment marker, the quotes its strings open with, and the
//! words it reserves.
//!
//! The table is data, and it is deliberately small. A model's answer is not a
//! compiler's problem: a reader wants the shape of the line — the keyword, the
//! string, the comment — and an unknown language, or a construct this table
//! does not know, paints plain rather than wrong. The set is the list a terminal
//! answer actually carries; a fence naming anything else stays plain.

/// One language a fence can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Sql,
    Shell,
    Json,
    Toml,
    Yaml,
    Go,
    Java,
}

/// What one language's lines are read with.
pub(super) struct Dialect {
    /// The words it reserves, matched whole and (in SQL) without case.
    pub(super) keywords: &'static [&'static str],
    /// The marker a comment starts with, if the language has one.
    pub(super) comment: Option<&'static str>,
    /// The characters a string opens with.
    pub(super) quotes: &'static [char],
    /// Whether the language's keywords are matched without case.
    pub(super) caseless: bool,
}

/// The language a fence's word names, or `None` for one this table does not
/// know. Aliases are the spellings models write: `rs`, `py`, `sh`, `yml`.
pub(crate) fn language(name: &str) -> Option<Language> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => Language::Rust,
        "python" | "py" => Language::Python,
        "javascript" | "js" | "jsx" => Language::JavaScript,
        "typescript" | "ts" | "tsx" => Language::TypeScript,
        "sql" => Language::Sql,
        "shell" | "sh" | "bash" | "zsh" | "console" => Language::Shell,
        "json" => Language::Json,
        "toml" => Language::Toml,
        "yaml" | "yml" => Language::Yaml,
        "go" => Language::Go,
        "java" => Language::Java,
        _ => return None,
    })
}

/// The words and markers one language is read with.
pub(super) fn dialect(language: Language) -> Dialect {
    match language {
        Language::Rust => Dialect {
            keywords: RUST,
            comment: Some("//"),
            quotes: &['"', '\''],
            caseless: false,
        },
        Language::Python => Dialect {
            keywords: PYTHON,
            comment: Some("#"),
            quotes: &['"', '\''],
            caseless: false,
        },
        Language::JavaScript | Language::TypeScript => Dialect {
            keywords: SCRIPT,
            comment: Some("//"),
            quotes: &['"', '\'', '`'],
            caseless: false,
        },
        Language::Sql => Dialect {
            keywords: SQL,
            comment: Some("--"),
            quotes: &['\'', '"'],
            caseless: true,
        },
        Language::Shell => Dialect {
            keywords: SHELL,
            comment: Some("#"),
            quotes: &['"', '\''],
            caseless: false,
        },
        Language::Json => Dialect {
            keywords: JSON,
            comment: None,
            quotes: &['"'],
            caseless: false,
        },
        Language::Toml => Dialect {
            keywords: BOOLEANS,
            comment: Some("#"),
            quotes: &['"', '\''],
            caseless: false,
        },
        Language::Yaml => Dialect {
            keywords: YAML,
            comment: Some("#"),
            quotes: &['"', '\''],
            caseless: false,
        },
        Language::Go => Dialect {
            keywords: GO,
            comment: Some("//"),
            quotes: &['"', '\'', '`'],
            caseless: false,
        },
        Language::Java => Dialect {
            keywords: JAVA,
            comment: Some("//"),
            quotes: &['"', '\''],
            caseless: false,
        },
    }
}

use super::highlight_keywords::{BOOLEANS, GO, JAVA, JSON, PYTHON, RUST, SCRIPT, SHELL, SQL, YAML};
