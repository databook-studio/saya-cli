//! JSON export: duplicate column labels are disambiguated so every value
//! stays addressable, then rows serialise as pretty objects.
//!
//! The whole array serialises through the bounded sink
//! ([`serde_json::to_writer_pretty`]), so the bytes are the serializer's own
//! and the ceiling is checked while it writes — an oversize export aborts
//! mid-stream instead of materialising in full.

use saya_types::QueryResult;
use serde::ser::{Serialize, SerializeMap, Serializer};

use super::shared::{BoundedWriter, EXPORT_TOO_LARGE, normalize_row};

struct JsonRow<'a> {
    columns: &'a [String],
    cells: &'a [serde_json::Value],
}

/// Renames repeated column labels so every key in one row is distinct:
/// the first `name` stays `name`; later repeats take `name_2`, `name_3`,
/// and so on. A repeated label must stay addressable after parsing —
/// duplicate object keys keep only the last value under ordinary JSON
/// parsing, silently dropping the earlier ones. All original labels are
/// reserved up front, so a generated suffix skips every name the query
/// itself used and an explicit `name_2` column is never shadowed.
pub(crate) fn disambiguated_columns(columns: &[String]) -> Vec<String> {
    let reserved: std::collections::HashSet<&str> = columns.iter().map(String::as_str).collect();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    columns
        .iter()
        .map(|column| {
            let next = counts.get(column.as_str()).copied().unwrap_or(0) + 1;
            counts.insert(column.as_str(), next);
            let mut candidate;
            if next == 1 {
                candidate = column.clone();
            } else {
                candidate = format!("{column}_{next}");
            }
            while seen.contains(&candidate)
                || (candidate != *column && reserved.contains(candidate.as_str()))
            {
                let bumped = counts.get(column.as_str()).copied().unwrap_or(1) + 1;
                counts.insert(column.as_str(), bumped);
                candidate = format!("{column}_{bumped}");
            }
            seen.insert(candidate.clone());
            candidate
        })
        .collect()
}

impl Serialize for JsonRow<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let names = disambiguated_columns(self.columns);
        let mut object = serializer.serialize_map(Some(names.len()))?;
        for (column, cell) in names.iter().zip(self.cells) {
            object.serialize_entry(column, cell)?;
        }
        object.end()
    }
}

/// Encodes the result as pretty JSON bytes under the ceiling, stopping
/// mid-stream when the sink refuses a write.
pub(super) fn encode_json(result: &QueryResult, ceiling: usize) -> Result<Vec<u8>, String> {
    let col_count = result.columns.len();
    let rows = result
        .rows
        .iter()
        .map(|row| normalize_row(row, col_count))
        .collect::<Vec<_>>();
    let objects = rows
        .iter()
        .map(|cells| JsonRow {
            columns: &result.columns,
            cells,
        })
        .collect::<Vec<_>>();
    let mut out = BoundedWriter::new(ceiling);
    let serialized = serde_json::to_writer_pretty(&mut out, &objects);
    if out.exceeded() {
        return Err(EXPORT_TOO_LARGE.into());
    }
    serialized.map_err(|e| format!("failed to serialize JSON: {e}"))?;
    Ok(out.into_inner())
}
