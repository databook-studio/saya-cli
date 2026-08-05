use std::collections::BTreeSet;

use saya_types::SchemaTree;

use super::complete::Candidate;

/// Flattens a database schema into a sorted, de-duplicated list of reference
/// strings (`table` and `table.column`).
#[allow(dead_code)]
pub(crate) fn schema_refs(schema: &SchemaTree) -> Vec<String> {
    let mut refs = BTreeSet::new();
    for db in &schema.databases {
        for s in &db.schemas {
            for t in &s.tables {
                refs.insert(t.name.clone());
                for c in &t.columns {
                    refs.insert(format!("{}.{}", t.name, c.name));
                }
            }
        }
    }
    refs.into_iter().collect()
}

/// Computes `@`-reference autocomplete candidates for a prompt input line.
///
/// Looks at the prefix of `line` up to `cursor_char`. If a valid `@`-reference
/// fragment is being typed, returns `(at_index, cursor_char, candidates)`
/// where `at_index` is the char index of `@`.
#[allow(dead_code)]
pub(crate) fn at_candidates(
    line: &str,
    cursor_char: usize,
    refs: &[String],
) -> Option<(usize, usize, Vec<Candidate>)> {
    let chars: Vec<char> = line.chars().collect();
    let cursor_char = cursor_char.min(chars.len());
    let prefix = &chars[..cursor_char];

    let at_index = prefix.iter().rposition(|&c| c == '@')?;
    let token = &prefix[at_index + 1..];

    if token.iter().any(|c| c.is_whitespace()) {
        return None;
    }

    if token.iter().filter(|&&c| c == '.').count() > 1 {
        return None;
    }

    if token
        .iter()
        .any(|&c| !c.is_alphanumeric() && c != '_' && c != '.')
    {
        return None;
    }

    let token_str: String = token.iter().collect();

    let mut scored: Vec<(i32, Candidate)> = refs
        .iter()
        .filter_map(|r| {
            super::fuzzy::fuzzy_score(r, &token_str).map(|score| {
                (
                    score,
                    Candidate {
                        value: format!("@{r}"),
                        description: None,
                    },
                )
            })
        })
        .collect();

    if scored.is_empty() {
        return None;
    }
    scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let candidates = scored.into_iter().map(|(_, candidate)| candidate).collect();

    Some((at_index, cursor_char, candidates))
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::{Column, Database, Schema, Table};

    fn sample_schema() -> SchemaTree {
        SchemaTree {
            databases: vec![
                Database {
                    name: "db1".to_string(),
                    schemas: vec![Schema {
                        name: "public".to_string(),
                        tables: vec![
                            Table {
                                name: "orders".to_string(),
                                columns: vec![
                                    Column {
                                        name: "id".to_string(),
                                        data_type: "int".to_string(),
                                        nullable: false,
                                    },
                                    Column {
                                        name: "total".to_string(),
                                        data_type: "numeric".to_string(),
                                        nullable: true,
                                    },
                                ],
                            },
                            Table {
                                name: "users".to_string(),
                                columns: vec![Column {
                                    name: "id".to_string(),
                                    data_type: "int".to_string(),
                                    nullable: false,
                                }],
                            },
                        ],
                    }],
                },
                Database {
                    name: "db2".to_string(),
                    schemas: vec![Schema {
                        name: "public".to_string(),
                        tables: vec![Table {
                            name: "orders".to_string(),
                            columns: vec![Column {
                                name: "id".to_string(),
                                data_type: "int".to_string(),
                                nullable: false,
                            }],
                        }],
                    }],
                },
            ],
        }
    }

    #[test]
    fn test_schema_refs() {
        let schema = sample_schema();
        let refs = schema_refs(&schema);
        assert_eq!(
            refs,
            vec!["orders", "orders.id", "orders.total", "users", "users.id"]
        );
    }

    #[test]
    fn test_at_candidates_matching() {
        let refs = vec![
            "orders".to_string(),
            "orders.id".to_string(),
            "users".to_string(),
        ];
        let res = at_candidates("show @ord", 8, &refs).unwrap();
        assert_eq!(res.0, 5);
        assert_eq!(res.1, 8);
        let vals: Vec<&str> = res.2.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(vals, vec!["@orders", "@orders.id"]);
    }

    #[test]
    fn test_at_alone() {
        let refs = vec!["orders".to_string(), "users".to_string()];
        let res = at_candidates("@", 1, &refs).unwrap();
        assert_eq!((res.0, res.1), (0, 1));
        assert_eq!(res.2.len(), 2);
    }

    #[test]
    fn test_no_at_prefix() {
        let refs = vec!["orders".to_string()];
        assert_eq!(at_candidates("show orders", 11, &refs), None);
    }

    #[test]
    fn test_at_followed_by_space() {
        let refs = vec!["orders".to_string()];
        assert_eq!(at_candidates("@ ", 2, &refs), None);
    }

    #[test]
    fn test_case_insensitive() {
        let refs = vec!["orders".to_string()];
        let res = at_candidates("show @ORD", 9, &refs).unwrap();
        assert_eq!(res.2[0].value, "@orders");
    }

    #[test]
    fn test_multibyte_prefix() {
        let refs = vec!["orders".to_string()];
        let res = at_candidates("🦀 @ord", 6, &refs).unwrap();
        assert_eq!(res.0, 2);
        assert_eq!(res.1, 6);
        assert_eq!(res.2[0].value, "@orders");
    }
}
