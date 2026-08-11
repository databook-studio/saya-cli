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
}
