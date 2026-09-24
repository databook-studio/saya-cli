//! Bounded RFC 4180 CSV parsing for the scratch importer.

use std::collections::{HashMap, HashSet};

use thiserror::Error;

/// The largest admitted field, measured in UTF-8 bytes.
pub const MAX_CSV_FIELD_BYTES: usize = 64 * 1024;
/// The most columns an imported table may have.
pub const MAX_CSV_COLUMNS: usize = 512;
/// The most data rows an import may add.
pub const MAX_CSV_ROWS: usize = 500_000;

/// A CSV refusal that never includes an imported field value.
#[derive(Debug, Error)]
pub enum CsvError {
    #[error("CSV is not valid UTF-8")]
    InvalidUtf8,
    #[error("CSV delimiter must be one ASCII byte")]
    InvalidDelimiter,
    #[error("CSV has invalid quote syntax on line {line}")]
    InvalidQuote { line: usize },
    #[error("CSV field on line {line} exceeds the {max}-byte limit")]
    FieldTooLarge { line: usize, max: usize },
    #[error("CSV has more than the {max} column limit on line {line}")]
    TooManyColumns { line: usize, max: usize },
    #[error("CSV has more than the {max} data-row limit")]
    TooManyRows { max: usize },
    #[error("CSV has an unterminated quoted field starting on line {line}")]
    UnterminatedQuote { line: usize },
}

pub(crate) struct CsvRow {
    pub(crate) line: usize,
    pub(crate) fields: Vec<String>,
}

/// Parses the RFC 4180 subset accepted by the importer: CRLF/LF records,
/// quoted fields, doubled quotes, and quoted embedded newlines.
pub fn parse_csv(bytes: &[u8], delimiter: u8) -> Result<Vec<Vec<String>>, CsvError> {
    Ok(parse_csv_rows(bytes, delimiter)?
        .into_iter()
        .map(|row| row.fields)
        .collect())
}

pub(crate) fn parse_csv_rows(bytes: &[u8], delimiter: u8) -> Result<Vec<CsvRow>, CsvError> {
    if !delimiter.is_ascii()
        || matches!(delimiter, b'"' | b'\r' | b'\n')
        || delimiter.is_ascii_control()
    {
        return Err(CsvError::InvalidDelimiter);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| CsvError::InvalidUtf8)?;
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut row_line = 1;
    let mut field = String::new();
    let mut quoted = false;
    let mut quote_line = 1;
    let mut after_quote = false;
    let mut line = 1;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    after_quote = true;
                }
            } else {
                if ch == '\n' {
                    line += 1;
                }
                field.push(ch);
            }
        } else {
            match ch {
                '"' if field.is_empty() && !after_quote => {
                    quoted = true;
                    quote_line = line;
                }
                '"' => return Err(CsvError::InvalidQuote { line }),
                c if c as u32 == delimiter as u32 => {
                    push_field(&mut row, &mut field, line)?;
                    after_quote = false;
                }
                '\n' => {
                    push_field(&mut row, &mut field, line)?;
                    push_row(&mut rows, std::mem::take(&mut row), row_line)?;
                    line += 1;
                    row_line = line;
                    after_quote = false;
                }
                '\r' if chars.peek() == Some(&'\n') => {
                    chars.next();
                    push_field(&mut row, &mut field, line)?;
                    push_row(&mut rows, std::mem::take(&mut row), row_line)?;
                    line += 1;
                    row_line = line;
                    after_quote = false;
                }
                '\r' => field.push(ch),
                _ if after_quote => return Err(CsvError::InvalidQuote { line }),
                _ => field.push(ch),
            }
        }
        if field.len() > MAX_CSV_FIELD_BYTES {
            return Err(CsvError::FieldTooLarge {
                line,
                max: MAX_CSV_FIELD_BYTES,
            });
        }
    }
    if quoted {
        return Err(CsvError::UnterminatedQuote { line: quote_line });
    }
    if !field.is_empty() || !row.is_empty() || after_quote {
        push_field(&mut row, &mut field, line)?;
        push_row(&mut rows, row, row_line)?;
    }
    Ok(rows)
}

fn push_field(row: &mut Vec<String>, field: &mut String, line: usize) -> Result<(), CsvError> {
    row.push(std::mem::take(field));
    if row.len() > MAX_CSV_COLUMNS {
        return Err(CsvError::TooManyColumns {
            line,
            max: MAX_CSV_COLUMNS,
        });
    }
    Ok(())
}

fn push_row(rows: &mut Vec<CsvRow>, row: Vec<String>, line: usize) -> Result<(), CsvError> {
    rows.push(CsvRow { line, fields: row });
    if rows.len() > MAX_CSV_ROWS.saturating_add(1) {
        return Err(CsvError::TooManyRows { max: MAX_CSV_ROWS });
    }
    Ok(())
}

/// Makes CSV header names safe, nonempty DuckDB identifiers, adding a stable
/// numeric suffix when sanitisation makes names collide.
pub fn sanitize_headers(headers: &[String]) -> Vec<String> {
    let mut seen = HashMap::<String, usize>::new();
    let mut used = HashSet::new();
    headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            let base = sanitise(header, index + 1);
            let count = seen.entry(base.clone()).or_insert(0);
            loop {
                *count += 1;
                let candidate = if *count == 1 {
                    base.clone()
                } else {
                    format!("{base}_{count}")
                };
                if used.insert(candidate.clone()) {
                    return candidate;
                }
            }
        })
        .collect()
}

fn sanitise(header: &str, position: usize) -> String {
    let mut name: String = header
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        return format!("column_{position}");
    }
    if !name.as_bytes()[0].is_ascii_alphabetic() && name.as_bytes()[0] != b'_' {
        name.insert(0, '_');
    }
    // Leave enough room for the deterministic `_512` collision suffix.
    name.truncate(59);
    name
}
