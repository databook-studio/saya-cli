//! Scratch's public vocabulary: the validated statement and its typed
//! rejections, and the row and byte caps the policy enforces.

/// Rows a scratch result may hand back to the model — the same small-sample
/// discipline as the interactive SQL tools: a larger result is truncated
/// (`truncated: true`) rather than shipped whole.
pub const SCRATCH_ROW_CAP: usize = 50;

/// Maximum SQL payload parsed by the scratch validator. The tool input is
/// untrusted; this pre-parse ceiling keeps the parser from being an allocation
/// bypass for callers that do not use the CLI input reader.
pub const MAX_SQL_BYTES: usize = 512 * 1024;

/// A statement the validator accepted, ready to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    /// The statement as it will run; reads carry an injected `LIMIT` when the statement did not bound itself.
    pub sql: String,
    /// True when the statement hands rows back (SELECT, EXPLAIN, SHOW,
    /// `INSERT … RETURNING`); false when it returns no rows.
    pub returns_rows: bool,
}

/// Why scratch refused a statement. Typed — the tool surfaces the variant's
/// own text so the model can self-correct — and deliberately its own type:
/// the connector's rejection vocabulary belongs to the user-database gate,
/// and scratch's does not parameterise it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ScratchRejection {
    #[error("scratch SQL exceeds the {max}-byte limit (received {bytes} bytes)")]
    TooLarge { bytes: usize, max: usize },
    #[error("scratch accepts exactly one statement; {count} were given — run them one at a time")]
    MultipleStatements { count: usize },
    #[error("scratch does not parse this as a single DuckDB statement")]
    Unparsable,
    #[error(
        "scratch refuses the file-reading function \"{0}\" — no file reader of any kind is allowed inside scratch; stage the data through the workspace tools instead"
    )]
    FileFunction(String),
    #[error(
        "scratch refuses \"{0}\" as a table name — a quoted file path is not a relation, and no file reads are allowed inside scratch"
    )]
    FilePathRelation(String),
    #[error(
        "scratch refuses {0} — the scratch engine holds no other files, no extensions, no attachments, and no configuration"
    )]
    Statement(String),
}
