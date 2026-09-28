//! The staged-source preview: the resolved delimiter, per-column null counts
//! over every data row, inferred types over the sampled rows, and up to five
//! sample rows of the parsed CSV fields.

use super::infer::{InferredType, infer};

/// The most sample rows a preview carries.
pub const MAX_SAMPLE_ROWS: usize = 5;

/// One column's preview: its header name, how many data-row fields were
/// empty (staged as NULL), and the sampled inferred type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewColumn {
    pub name: String,
    pub null_count: usize,
    pub inferred: InferredType,
}

/// The staged CSV's preview. `delimiter` is the delimiter actually used —
/// the caller's choice, or the sniffed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub delimiter: u8,
    pub header: bool,
    pub columns: Vec<PreviewColumn>,
    pub sample_rows: Vec<Vec<String>>,
}

/// Resolves the staging delimiter: the caller's explicit byte, or a sniff
/// over the source's first line. An explicit byte is not re-validated here —
/// the CSV parser refuses unsafe delimiters on the parse.
pub(super) fn resolve_delimiter(option: Option<u8>, bytes: &[u8]) -> u8 {
    option.unwrap_or_else(|| sniff(bytes))
}

/// Sniffs a delimiter from the source's first line (up to 64 KiB): the most
/// frequent of the four candidate delimiters wins; ties favour comma, then
/// semicolon, then tab, then pipe; none present → comma. Quoted fields are
/// not exempted — callers with quoting-sensitive input pass an explicit
/// delimiter.
fn sniff(bytes: &[u8]) -> u8 {
    let first_line = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len())
        .min(64 * 1024);
    let line = &bytes[..first_line];
    // Reversed so `max_by_key` (last maximum wins) breaks ties in favour of
    // the earlier preference: comma, semicolon, tab, pipe.
    const CANDIDATES: [u8; 4] = *b"|\t;,";
    CANDIDATES
        .into_iter()
        .max_by_key(|candidate| line.iter().filter(|byte| *byte == candidate).count())
        .unwrap_or(b',')
}

pub(super) fn build(
    delimiter: u8,
    header: bool,
    columns: &[String],
    rows: &[Vec<String>],
) -> Preview {
    let inferred = infer(rows, columns.len());
    let columns = columns
        .iter()
        .enumerate()
        .map(|(index, name)| PreviewColumn {
            name: name.clone(),
            null_count: rows
                .iter()
                .filter(|row| row.get(index).is_none_or(String::is_empty))
                .count(),
            inferred: inferred[index],
        })
        .collect();
    let sample_rows = rows.iter().take(MAX_SAMPLE_ROWS).cloned().collect();
    Preview {
        delimiter,
        header,
        columns,
        sample_rows,
    }
}
