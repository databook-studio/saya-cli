//! Heuristic chart specification suggestions.

use saya_types::QueryResult;

use super::{ChartKind, ChartSpec, is_numeric_column, normalize_row};

/// Heuristic default when the caller doesn't specify how to chart `result`.
#[allow(dead_code)]
pub(crate) fn suggest_spec(result: &QueryResult) -> ChartSpec {
    let col_count = result.columns.len();
    if col_count == 0 || result.rows.is_empty() {
        return ChartSpec {
            kind: ChartKind::Bar,
            x: None,
            y: Vec::new(),
            title: None,
        };
    }

    let normalized_rows: Vec<Vec<serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| normalize_row(row, col_count))
        .collect();

    let numeric: Vec<usize> = (0..col_count)
        .filter(|&idx| is_numeric_column(&normalized_rows, idx))
        .collect();
    let categorical: Vec<usize> = (0..col_count)
        .filter(|idx| !numeric.contains(idx))
        .collect();

    if col_count == 2 && numeric.len() == 2 {
        ChartSpec {
            kind: ChartKind::Scatter,
            x: Some(result.columns[0].clone()),
            y: vec![result.columns[1].clone()],
            title: None,
        }
    } else if !categorical.is_empty() && !numeric.is_empty() {
        ChartSpec {
            kind: ChartKind::Bar,
            x: Some(result.columns[categorical[0]].clone()),
            y: vec![result.columns[numeric[0]].clone()],
            title: None,
        }
    } else {
        let x_col = result.columns[0].clone();
        let y_col = if let Some(&idx) = numeric.first() {
            result.columns[idx].clone()
        } else if result.columns.len() > 1 {
            result.columns[1].clone()
        } else {
            result.columns[0].clone()
        };
        ChartSpec {
            kind: ChartKind::Bar,
            x: Some(x_col),
            y: vec![y_col],
            title: None,
        }
    }
}
