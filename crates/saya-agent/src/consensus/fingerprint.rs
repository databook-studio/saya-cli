use saya_types::QueryResult;
use serde_json::{Map, Number, Value};

/// Canonical string for a result set.
///
/// Two results that represent the same answer produce the same fingerprint.
/// The fingerprint is a deterministic JSON array `[columns, rows]`:
///
/// - Columns are included in order, so a different name or count changes the
///   fingerprint.
/// - When `ordered` is false, rows are sorted canonically before hashing so the
///   same rows in a different order match; when true, row order is preserved.
/// - Cell values are normalised so trivial representation differences do not
///   split a group (integral floats become integers, booleans stay `true` /
///   `false`, `null` is distinct from `""` and the string `"NULL"`).
/// - Truncation is part of the answer. A result cut off at the row cap is not
///   the same answer as a complete one that happens to hold those rows, and two
///   truncated results agreeing says only that their retained prefixes agree —
///   so the flag is folded in rather than ignored, and the caller can see that
///   a winning group was built from partial results.
///
/// JSON serialisation gives correct, collision-free escaping for free; a
/// `serde_json::Value` cannot hold non-finite floats or non-string map keys,
/// so serialising the normalised structure is infallible.
pub fn fingerprint(result: &QueryResult, ordered: bool) -> String {
    let columns = Value::Array(
        result
            .columns
            .iter()
            .map(|c| Value::String(c.clone()))
            .collect(),
    );

    let mut row_reprs: Vec<String> = result.rows.iter().map(serialize_row).collect();
    if !ordered {
        row_reprs.sort_unstable();
    }

    let cols_json =
        serde_json::to_string(&columns).expect("Value::Array<String> serialises infallibly");
    let rows_json = format!("[{}]", row_reprs.join(","));
    format!("[{cols_json},{rows_json},{}]", result.truncated)
}

/// Serialise a single row to canonical JSON after normalising each cell.
fn serialize_row(row: &Value) -> String {
    let cells = match row {
        Value::Array(arr) => arr.iter().map(normalize_cell).collect(),
        other => vec![normalize_cell(other)],
    };
    serde_json::to_string(&Value::Array(cells))
        .expect("normalised cells serialise infallibly; Value excludes non-finite floats")
}

/// Normalise a single cell value to its canonical form.
fn normalize_cell(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Number(n) => normalize_number(n),
        Value::String(s) => Value::String(s.clone()),
        Value::Array(arr) => Value::Array(arr.iter().map(normalize_cell).collect()),
        Value::Object(obj) => Value::Object(normalize_object(obj)),
    }
}

/// Normalise an object's values (keys are already strings; order is sorted by
/// `serde_json::Map`, giving a deterministic representation).
fn normalize_object(obj: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in obj {
        out.insert(k.clone(), normalize_cell(v));
    }
    out
}

/// Normalise a JSON number.
///
/// Integers keep their canonical form. Integral floats within the exactly
/// representable range collapse to integers so engines that report `3` and
/// `3.0` agree; non-integral or out-of-range floats keep full precision.
fn normalize_number(n: &Number) -> Value {
    if n.is_i64() || n.is_u64() {
        return Value::Number(n.clone());
    }
    let Some(f) = n.as_f64() else {
        return Value::Number(n.clone());
    };
    // Integers up to 2^53 are exactly representable as f64; the cast is exact.
    const EXACT_INT_BOUND: f64 = (1u64 << 53) as f64;
    if f.is_finite() && f.fract() == 0.0 && f.abs() <= EXACT_INT_BOUND {
        return Value::from(f as i64);
    }
    Value::Number(n.clone())
}
