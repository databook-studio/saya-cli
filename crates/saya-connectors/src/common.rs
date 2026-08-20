use serde_json::Value;

pub(crate) const MAX_CELL_BYTES: usize = 1 << 20;
pub(crate) const MAX_RESULT_BYTES: usize = 16 << 20;

pub(crate) fn cap_cell(v: Value) -> Value {
    match v {
        Value::String(s) => {
            if s.len() > MAX_CELL_BYTES {
                let mut cut = MAX_CELL_BYTES;
                while !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                let truncated_bytes = s.len() - cut;
                let slice = &s[..cut];
                Value::String(format!("{slice}…[truncated {truncated_bytes} bytes]"))
            } else {
                Value::String(s)
            }
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(cap_cell).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, cap_cell(v))).collect())
        }
        other => other,
    }
}

pub(crate) fn value_bytes(v: &Value) -> usize {
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => 8,
        Value::String(s) => s.len(),
        Value::Array(arr) => arr.iter().map(value_bytes).sum(),
        Value::Object(map) => map.iter().map(|(k, v)| k.len() + value_bytes(v)).sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cap_cell_small_string_unchanged() {
        let small = Value::String("hello world".to_string());
        assert_eq!(cap_cell(small.clone()), small);
    }

    #[test]
    fn test_cap_cell_oversized_string_truncated_on_char_boundary() {
        let large_ascii = "a".repeat(MAX_CELL_BYTES + 500);
        let res = cap_cell(Value::String(large_ascii));
        if let Value::String(s) = res {
            assert!(s.starts_with(&"a".repeat(MAX_CELL_BYTES)));
            assert!(s.contains("…[truncated 500 bytes]"));
        } else {
            panic!("Expected Value::String");
        }

        let mut string_with_multibyte = "a".repeat(MAX_CELL_BYTES - 2);
        string_with_multibyte.push_str("🦀🦀🦀");
        let res_mb = cap_cell(Value::String(string_with_multibyte));
        if let Value::String(s) = res_mb {
            assert!(s.contains("…[truncated"));
        } else {
            panic!("Expected Value::String");
        }
    }

    #[test]
    fn test_cap_cell_non_string_unchanged() {
        assert_eq!(cap_cell(Value::Null), Value::Null);
        assert_eq!(cap_cell(Value::Bool(true)), Value::Bool(true));
        assert_eq!(cap_cell(Value::from(42)), Value::from(42));
    }

    #[test]
    fn test_value_bytes() {
        assert_eq!(value_bytes(&Value::Null), 8);
        assert_eq!(value_bytes(&Value::Bool(false)), 8);
        assert_eq!(value_bytes(&Value::from(100)), 8);
        assert_eq!(value_bytes(&Value::String("hello".to_string())), 5);

        let arr = Value::Array(vec![Value::String("abc".to_string()), Value::from(10)]);
        assert_eq!(value_bytes(&arr), 3 + 8);

        let mut map = serde_json::Map::new();
        map.insert("key".to_string(), Value::String("val".to_string()));
        let obj = Value::Object(map);
        assert_eq!(value_bytes(&obj), 3 + 3);
    }

    #[test]
    fn test_cap_cell_boundary_exact_and_multibyte() {
        // String at EXACTLY MAX_CELL_BYTES (1 MiB) must remain unchanged
        let exact_ascii = "x".repeat(MAX_CELL_BYTES);
        let res_exact = cap_cell(Value::String(exact_ascii.clone()));
        if let Value::String(s) = res_exact {
            assert_eq!(s.len(), MAX_CELL_BYTES);
            assert_eq!(s, exact_ascii);
            assert!(!s.contains("…[truncated"));
        } else {
            panic!("Expected Value::String");
        }

        // String just over MAX_CELL_BYTES by 1 byte must be truncated
        let over_ascii = "x".repeat(MAX_CELL_BYTES + 1);
        let res_over = cap_cell(Value::String(over_ascii));
        if let Value::String(s) = res_over {
            assert!(s.starts_with(&"x".repeat(MAX_CELL_BYTES)));
            assert!(s.contains("…[truncated 1 bytes]"));
        } else {
            panic!("Expected Value::String");
        }

        // Multi-byte char crossing MAX_CELL_BYTES boundary:
        // (MAX_CELL_BYTES - 1) ASCII bytes + 4-byte '🦀' (MAX_CELL_BYTES + 3 total)
        let mut mb_str = "a".repeat(MAX_CELL_BYTES - 1);
        mb_str.push('🦀');
        let res_mb = cap_cell(Value::String(mb_str));
        if let Value::String(s) = res_mb {
            assert!(s.starts_with(&"a".repeat(MAX_CELL_BYTES - 1)));
            assert!(
                !s.contains('🦀'),
                "Multi-byte char straddling boundary must be safely truncated"
            );
            assert!(s.contains("…[truncated 4 bytes]"));
        } else {
            panic!("Expected Value::String");
        }
    }

    #[test]
    fn test_result_budget_helper_truncation() {
        let mut result_bytes = 0;
        let mut truncated = false;
        let mut row_count = 0;

        let cell_str = "r".repeat(500_000);
        let cell_val = Value::String(cell_str);

        while !truncated {
            let row = Value::Array(vec![cell_val.clone(), Value::from(row_count)]);
            let row_bytes = value_bytes(&row);
            result_bytes += row_bytes;
            row_count += 1;

            if result_bytes > MAX_RESULT_BYTES {
                truncated = true;
            }
        }

        assert!(
            truncated,
            "Bounded accumulation must report truncated once MAX_RESULT_BYTES is crossed"
        );
        assert!(
            result_bytes > MAX_RESULT_BYTES,
            "Total bytes ({result_bytes}) must exceed MAX_RESULT_BYTES ({MAX_RESULT_BYTES})"
        );

        let max_allowed = MAX_RESULT_BYTES + 500_000 + 8;
        assert!(
            result_bytes <= max_allowed,
            "Accumulation must stop immediately after crossing budget, got {result_bytes} vs max allowed {max_allowed}"
        );
    }
}
