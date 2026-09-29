use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dialect::SqlDialect;
use crate::query::QueryResult;

/// Upper bound on knowledge references attached to one evidence record.
pub const MAX_EVIDENCE_KNOWLEDGE_IDS: usize = 64;

/// How much of a result the caller is allowed to see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResultScope {
    Full,
    ModelLimited { row_cap: usize },
}

/// Where an execution came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceSource {
    DirectSql,
    SavedInvestigation {
        id: String,
        revision: u32,
    },
    Agent,
    /// An execution served through `saya mcp serve` (ADR 0008): the query
    /// tool's evidence names the server as its source.
    Mcp,
}

/// Inputs for [`ExecutionEvidence::for_result`], keeping the constructor under
/// clippy's argument limit.
#[derive(Debug, Clone)]
pub struct ExecutionEvidenceArgs {
    pub execution_id: String,
    pub connection_label: String,
    pub connection_identity: Option<String>,
    pub dialect: SqlDialect,
    pub max_rows: usize,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
    pub source: EvidenceSource,
}

/// A durable record of one successful execution: the statement is named by
/// hash only, so serialized evidence never carries SQL text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionEvidence {
    pub execution_id: String,
    /// SHA-256 of the SQL the caller submitted, as 64 lowercase hex chars.
    pub submitted_sql_sha256: String,
    /// Profile label, safe to show; [`Self::connection_identity`] is not.
    pub connection_label: String,
    pub connection_identity: Option<String>,
    pub dialect: SqlDialect,
    pub schema_fingerprint: Option<String>,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
    pub max_rows: usize,
    pub returned_rows: usize,
    pub truncated: bool,
    pub scope: ResultScope,
    pub source: EvidenceSource,
    #[serde(default)]
    pub knowledge_ids: Vec<String>,
    /// The bound parameters' NAMES, in declaration order — never a value.
    /// Absent from serialization when empty (a parameter-free run).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_names: Vec<String>,
    /// SHA-256 over the canonical `name=value` lines of the bound
    /// parameters, in declaration order — a digest of the value set, never
    /// the values themselves. Absent when no parameters were bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_sha256: Option<String>,
}

impl ExecutionEvidence {
    /// Hashes the submitted SQL, copies counts and truncation, scope Full.
    pub fn for_result(result: &QueryResult, args: ExecutionEvidenceArgs) -> Self {
        Self {
            execution_id: args.execution_id,
            submitted_sql_sha256: sha256_hex(&result.executed_sql),
            connection_label: args.connection_label,
            connection_identity: args.connection_identity,
            dialect: args.dialect,
            schema_fingerprint: None,
            started_unix_ms: args.started_unix_ms,
            finished_unix_ms: args.finished_unix_ms,
            max_rows: args.max_rows,
            returned_rows: result.row_count,
            truncated: result.truncated,
            scope: ResultScope::Full,
            source: args.source,
            knowledge_ids: Vec::new(),
            param_names: Vec::new(),
            params_sha256: None,
        }
    }

    /// Attaches knowledge references, clamped to [`MAX_EVIDENCE_KNOWLEDGE_IDS`].
    pub fn with_knowledge_ids(mut self, mut ids: Vec<String>) -> Self {
        ids.truncate(MAX_EVIDENCE_KNOWLEDGE_IDS);
        self.knowledge_ids = ids;
        self
    }

    /// Attaches the bound parameters' names and the digest over their
    /// canonical `name=value` lines (B1): names and a hash only — the values
    /// themselves never reach evidence, transcripts, or reports.
    pub fn with_param_bindings(mut self, names: Vec<String>, digest: Option<String>) -> Self {
        self.param_names = names;
        self.params_sha256 = digest;
        self
    }

    /// `"x"` + base36(started unix ms) + `-` + base36(counter), at most 28 chars.
    pub fn new_execution_id(started_unix_ms: i64, counter: u64) -> String {
        let mut id = String::from("x");
        push_base36(started_unix_ms.unsigned_abs(), &mut id);
        id.push('-');
        push_base36(counter, &mut id);
        id
    }

    /// At most the first 12 characters of [`Self::execution_id`].
    pub fn short_id(&self) -> &str {
        match self.execution_id.char_indices().nth(12) {
            Some((idx, _)) => &self.execution_id[..idx],
            None => &self.execution_id,
        }
    }

    /// One transcript line: source, profile label, rows, truncation, short id,
    /// scope, and the bound parameters' names — never SQL text, never a
    /// value.
    pub fn human_line(&self) -> String {
        let rows = if self.truncated {
            format!(
                "{} rows (truncated at {})",
                self.returned_rows, self.max_rows
            )
        } else {
            format!("{} rows", self.returned_rows)
        };
        let source = match &self.source {
            EvidenceSource::DirectSql => "direct sql",
            EvidenceSource::SavedInvestigation { .. } => "saved investigation",
            EvidenceSource::Agent => "agent",
            EvidenceSource::Mcp => "mcp server",
        };
        let scope = match &self.scope {
            ResultScope::Full => "full result".to_owned(),
            ResultScope::ModelLimited { row_cap } => {
                format!("model-limited (first {row_cap} rows)")
            }
        };
        let params = if self.param_names.is_empty() {
            String::new()
        } else {
            format!(" · params: {}", self.param_names.join(", "))
        };
        format!(
            "{}: {} · {} · exec {} · {}{}",
            source,
            self.connection_label,
            rows,
            self.short_id(),
            scope,
            params
        )
    }
}

fn sha256_hex(input: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(input.as_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn push_base36(mut value: u64, out: &mut String) {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = [0u8; 13]; // u64::MAX is 13 base-36 digits.
    let mut len = 0;
    loop {
        buf[len] = DIGITS[(value % 36) as usize];
        value /= 36;
        len += 1;
        if value == 0 {
            break;
        }
    }
    out.extend(buf[..len].iter().rev().map(|&digit| digit as char));
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
