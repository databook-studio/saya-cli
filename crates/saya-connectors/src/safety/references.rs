//! Deterministic object and column extraction from a parsed SQL statement.
//!
//! See `.claude/specs/spec-3b1-sql-object-extraction.md`. The output is names
//! only — no SQL is persisted or returned — which is the property that lets a
//! caller store the result as an observation.

use saya_types::SqlDialect;
use sqlparser::ast::{Statement, Visit};
use sqlparser::parser::Parser;

use super::read_only::parser_dialect;

/// At most this many objects/columns are recorded; more sets [`SqlReferences::partial`].
const MAX_OBJECTS: usize = 64;
const MAX_COLUMNS: usize = 256;

/// Objects and columns referenced by a single SQL statement, as written.
///
/// `partial` is `true` when the statement parsed but used a construct this
/// extractor does not model, so `objects`/`columns` may be incomplete. A caller
/// must treat a partial result as "at least these, possibly more" and never as
/// an exhaustive list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SqlReferences {
    /// Table-ish names as written, each split into its parts (1, 2 or 3).
    pub objects: Vec<Vec<String>>,
    /// For each part of each object, `true` when the SQL quoted it (`"x"`,
    /// `` `x` ``, `[x]`) — the engine then matches that identifier exactly
    /// rather than applying its unquoted folding rules. Same shape as
    /// `objects`.
    pub object_quoting: Vec<Vec<bool>>,
    /// Column identifiers as written. Unqualified names appear bare.
    pub columns: Vec<String>,
    /// `true` when parsing succeeded but the statement used a construct this
    /// extractor does not model, so the lists may be incomplete.
    pub partial: bool,
}

/// Extract the object and column names a single SQL statement references.
///
/// Returns `None` when the SQL does not parse; a caller must not guess from
/// unparseable input. Parsed but unmodelled constructs set `partial = true`
/// rather than emitting a name the extractor is unsure about.
pub fn sql_references(sql: &str, dialect: SqlDialect) -> Option<SqlReferences> {
    let mut statements = Parser::parse_sql(parser_dialect(dialect), sql).ok()?;
    // A caller attributes one observation to one statement; multiple statements
    // in one string is ambiguous, so refuse rather than merge.
    if statements.len() != 1 {
        return None;
    }
    let query = match statements.remove(0) {
        Statement::Query(q) => q,
        // `EXPLAIN <query>` extracts from the underlying query; `EXPLAIN` of
        // anything else is parsed but not modelled.
        Statement::Explain { statement, .. } => match *statement {
            Statement::Query(q) => q,
            _ => {
                return Some(SqlReferences {
                    partial: true,
                    ..SqlReferences::default()
                });
            }
        },
        // DML/DDL/SHOW parse but are not modelled: empty lists, partial.
        _ => {
            return Some(SqlReferences {
                partial: true,
                ..SqlReferences::default()
            });
        }
    };
    let mut ex = Extractor::default();
    // The visitor never breaks, so a `Break` here would mean we stopped before
    // seeing the whole tree — the lists may be incomplete.
    let stopped_early = query.visit(&mut ex).is_break();
    Some(SqlReferences {
        objects: ex.objects,
        object_quoting: ex.object_quoting,
        columns: ex.columns,
        partial: ex.partial || stopped_early,
    })
}

mod walk;
use walk::Extractor;

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::SqlDialect;

    /// Postgres accepts the most syntax (catalog-qualified names, CTEs), so it
    /// is the default dialect for these tests unless a case is dialect-specific.
    const D: SqlDialect = SqlDialect::Postgres;

    fn objects(sql: &str) -> Vec<Vec<String>> {
        sql_references(sql, D).expect("should parse").objects
    }
    fn columns(sql: &str) -> Vec<String> {
        sql_references(sql, D).expect("should parse").columns
    }
    fn partial(sql: &str) -> bool {
        sql_references(sql, D).expect("should parse").partial
    }
    fn quoting(sql: &str) -> Vec<Vec<bool>> {
        sql_references(sql, D).expect("should parse").object_quoting
    }

    #[test]
    fn catalog_qualified_name_keeps_three_parts() {
        // Spec case 1: catalog.schema.table → 3 parts, plus the column.
        let r = sql_references("SELECT id FROM analytics.public.orders", D).unwrap();
        assert_eq!(r.objects, vec![vec!["analytics", "public", "orders"]]);
        assert_eq!(r.columns, vec!["id".to_string()]);
        assert!(!r.partial);
    }

    #[test]
    fn two_part_and_one_part_names_have_no_invented_catalog() {
        // Spec case 2: only as many parts as written.
        assert_eq!(
            objects("SELECT a FROM public.orders"),
            vec![vec!["public", "orders"]]
        );
        assert_eq!(objects("SELECT a FROM orders"), vec![vec!["orders"]]);
    }

    #[test]
    fn join_reports_both_tables_in_first_seen_order() {
        // Spec case 3.
        assert_eq!(
            objects("SELECT a FROM orders o JOIN customers c ON o.cid = c.id"),
            vec![vec!["orders"], vec!["customers"]]
        );
    }

    #[test]
    fn subquery_table_is_reported() {
        // Spec case 4.
        assert_eq!(
            objects("SELECT x FROM (SELECT x FROM line_items) sub"),
            vec![vec!["line_items"]]
        );
    }

    #[test]
    fn cte_name_is_not_an_object() {
        // Spec case 5: `recent` is a CTE alias; only `orders` is a real table.
        assert_eq!(
            objects("WITH recent AS (SELECT * FROM orders) SELECT * FROM recent"),
            vec![vec!["orders"]]
        );
    }

    #[test]
    fn chained_cte_reference_excludes_inner_cte_names() {
        // A CTE referencing an earlier CTE: `b`'s body reads `a`, which is a CTE,
        // not a table. Only `orders` should survive.
        assert_eq!(
            objects("WITH a AS (SELECT * FROM orders), b AS (SELECT * FROM a) SELECT * FROM b"),
            vec![vec!["orders"]]
        );
    }

    #[test]
    fn table_alias_is_not_an_object() {
        // Spec case 6.
        assert_eq!(objects("SELECT * FROM orders o"), vec![vec!["orders"]]);
    }

    #[test]
    fn qualified_column_keeps_only_the_column_part() {
        // Spec case 7: `o.id` → column `id`. The qualifier `o` is an alias and
        // must never appear as a column. We keep the LAST identifier part only,
        // so `schema.table.col` would also reduce to `col` — consistent and
        // never invents a column from a schema/alias part.
        assert_eq!(columns("SELECT o.id FROM orders o"), vec!["id".to_string()]);
        assert!(
            !columns("SELECT o.id FROM orders o")
                .iter()
                .any(|c| c == "o")
        );
    }

    #[test]
    fn unparseable_sql_returns_none() {
        // Spec case 8.
        assert!(sql_references("SELECT FROM WHERE", D).is_none());
        assert!(sql_references("not sql at all :::", D).is_none());
    }

    #[test]
    fn case_is_preserved_exactly() {
        // Spec case 9: no folding or normalisation.
        let r = sql_references("SELECT ID FROM Orders", D).unwrap();
        assert_eq!(r.objects, vec![vec!["Orders"]]);
        assert_eq!(r.columns, vec!["ID".to_string()]);
    }

    #[test]
    fn output_is_deterministic_for_repeated_input() {
        // Spec case 10.
        let one = sql_references("SELECT a, b FROM t JOIN s ON t.x = s.y", D).unwrap();
        let two = sql_references("SELECT a, b FROM t JOIN s ON t.x = s.y", D).unwrap();
        assert_eq!(one, two);
        // Dedup preserves first-seen order even with repeats.
        let r = sql_references("SELECT a, a, b FROM t, t", D).unwrap();
        assert_eq!(r.columns, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(r.objects, vec![vec!["t"]]);
    }

    #[test]
    fn object_bound_truncates_and_sets_partial() {
        // Spec case 11: more than 64 objects → partial, truncated to 64.
        let mut from = String::from("SELECT 1 FROM ");
        for i in 0..70 {
            if i > 0 {
                from.push_str(", ");
            }
            from.push_str(&format!("t{i}"));
        }
        let r = sql_references(&from, D).unwrap();
        assert!(r.partial);
        assert_eq!(r.objects.len(), 64);
    }

    #[test]
    fn column_bound_truncates_and_sets_partial() {
        // Spec case 11 (columns): more than 256 columns → partial, truncated.
        let mut select = String::from("SELECT ");
        for i in 0..300 {
            if i > 0 {
                select.push_str(", ");
            }
            select.push_str(&format!("c{i}"));
        }
        select.push_str(" FROM t");
        let r = sql_references(&select, D).unwrap();
        assert!(r.partial);
        assert_eq!(r.columns.len(), 256);
    }

    #[test]
    fn string_literal_does_not_leak_into_names() {
        // Spec case 12: a single-quoted string literal is data, never a name,
        // and must not ride along in any object or column. (A double-quoted
        // identifier would be a real table name and is out of scope here.)
        let r = sql_references(
            "SELECT 'SENTINELLITERAL' AS label, name FROM users WHERE token = 'SENTINELLITERAL'",
            D,
        )
        .unwrap();
        let blob = format!("{r:?}");
        assert!(
            !blob.contains("SENTINELLITERAL"),
            "literal leaked into output: {blob}"
        );
        assert_eq!(r.objects, vec![vec!["users"]]);
        // `name` and `token` are columns; the aliased literal contributes no
        // column, and the string in the WHERE stays out of the column list.
        assert_eq!(r.columns, vec!["name".to_string(), "token".to_string()]);
    }

    #[test]
    fn set_operation_reports_all_sides() {
        assert_eq!(
            objects("SELECT a FROM t UNION SELECT a FROM u"),
            vec![vec!["t"], vec!["u"]]
        );
    }

    #[test]
    fn table_valued_function_is_not_an_object_but_flags_partial() {
        // generate_series(...) parses as a table factor with function args; its
        // name is a function, not a table, so it must not be reported as an
        // object and the result is partial (a real object may be hidden).
        let r = sql_references("SELECT * FROM generate_series(1, 10)", D).unwrap();
        assert!(
            r.objects.is_empty()
                || !r
                    .objects
                    .iter()
                    .any(|o| o == &vec!["generate_series".to_string()])
        );
        assert!(r.partial, "a table-valued function must set partial");
    }

    #[test]
    fn output_column_alias_is_not_a_column() {
        // `SELECT x AS y FROM t` reports column `x`, not the output alias `y`.
        assert_eq!(columns("SELECT x AS y FROM t"), vec!["x".to_string()]);
        assert!(!columns("SELECT x AS y FROM t").iter().any(|c| c == "y"));
    }

    #[test]
    fn clean_query_is_not_partial() {
        // A plain select with no unmodelled construct is fully extracted.
        assert!(!partial("SELECT id, name FROM users WHERE id = 1"));
        assert!(!partial("SELECT a FROM t JOIN s ON t.x = s.y"));
    }

    #[test]
    fn cte_defined_in_subquery_does_not_hide_outer_table_cross_join() {
        // A CTE is visible only inside the query that declares it. Here
        // `realtable` is declared inside the derived table; the outer
        // CROSS JOIN reads the base table of the same name and it must be
        // reported.
        let r = sql_references(
            "SELECT * FROM (WITH realtable AS (SELECT 1) SELECT * FROM realtable) sub \
             CROSS JOIN realtable",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, vec![vec!["realtable"]]);
        assert!(!r.partial);
    }

    #[test]
    fn cte_defined_in_subquery_does_not_hide_outer_table_comma_join() {
        // Same scope bug through the comma-join form.
        let r = sql_references(
            "SELECT * FROM (WITH realtable AS (SELECT 1) SELECT * FROM realtable) sub, realtable",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, vec![vec!["realtable"]]);
        assert!(!r.partial);
    }

    #[test]
    fn cte_in_nested_subquery_does_not_hide_outer_table() {
        // The CTE sits two levels down; the trailing `t` after the outer
        // derived table is the base table.
        let r = sql_references(
            "SELECT * FROM (SELECT * FROM (WITH t AS (SELECT 1) SELECT * FROM t) x) y, t",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, vec![vec!["t"]]);
        assert!(!r.partial);
    }

    #[test]
    fn plain_cte_reference_is_still_not_an_object() {
        // A CTE in the statement's own WITH hides its name for the whole
        // query: `t` is the CTE, not a base table.
        let r = sql_references("WITH t AS (SELECT 1) SELECT * FROM t", D).unwrap();
        assert_eq!(r.objects, Vec::<Vec<String>>::new());
        assert!(!r.partial);
    }

    #[test]
    fn cte_shadows_same_named_table_only_inside_its_scope() {
        // Within the WITH's subtree `t` resolves to the CTE, so the base
        // table of the same name is absent there; a differently-named table
        // beside it is still reported.
        assert_eq!(
            objects("WITH t AS (SELECT 1) SELECT * FROM t, u"),
            vec![vec!["u"]]
        );
        // A subquery in the main body is still inside the CTE's scope.
        assert_eq!(
            objects("WITH t AS (SELECT 1) SELECT * FROM (SELECT * FROM t) s, u"),
            vec![vec!["u"]]
        );
    }

    #[test]
    fn recursive_cte_self_reference_is_not_an_object() {
        // A recursive CTE referencing itself inside its own definition is a
        // CTE reference, not the base table.
        let r = sql_references(
            "WITH RECURSIVE t AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM t) SELECT * FROM t",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, Vec::<Vec<String>>::new());
        assert!(!r.partial);
    }

    #[test]
    fn nested_with_same_alias_at_two_levels() {
        // The same alias defined at two nested scopes: the inner `t` is the
        // inner CTE and the trailing `t` is the outer CTE; both are CTE
        // references, so nothing is reported. Leaving the inner scope must
        // not drop the outer definition.
        let r = sql_references(
            "WITH t AS (SELECT 1) SELECT * FROM (WITH t AS (SELECT 2) SELECT * FROM t) x, t",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, Vec::<Vec<String>>::new());
        assert!(!r.partial);
    }

    #[test]
    fn nonrecursive_cte_body_sees_the_base_table() {
        // A non-recursive CTE cannot see its own alias: the `orders` inside
        // the body is the base table and must be reported; the `orders` in
        // the main body is the CTE and must not be.
        let r = sql_references(
            "WITH orders AS (SELECT * FROM orders WHERE x > 1) SELECT * FROM orders",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, vec![vec!["orders"]]);
        assert!(!r.partial);
    }

    #[test]
    fn later_cte_alias_not_visible_in_earlier_body() {
        // CTE bodies see only earlier CTEs of the same WITH: `b` inside `a`'s
        // body is the base table, while the main body's `a` is the CTE.
        let r = sql_references(
            "WITH a AS (SELECT * FROM b), b AS (SELECT 1) SELECT * FROM a",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, vec![vec!["b"]]);
        assert!(!r.partial);
    }

    #[test]
    fn earlier_cte_visible_in_later_body() {
        // An earlier CTE's alias is visible in a later CTE's body.
        let r = sql_references(
            "WITH a AS (SELECT 1), b AS (SELECT * FROM a) SELECT * FROM b",
            D,
        )
        .unwrap();
        assert_eq!(r.objects, Vec::<Vec<String>>::new());
        assert!(!r.partial);
    }

    #[test]
    fn subquery_inside_nonrecursive_cte_body_sees_the_base_table() {
        // The derived subquery sits inside the CTE's own body, which cannot
        // see its own alias: `t` there is the base table, not the CTE.
        assert_eq!(
            objects("WITH t AS (SELECT * FROM (SELECT * FROM t) s) SELECT * FROM t"),
            vec![vec!["t"]]
        );
    }

    #[test]
    fn quoting_is_recorded_per_part() {
        // Re-audit R1: `quote_style` must survive extraction — it is the only
        // way the review can tell `Orders` (which the engine folds) from
        // `"Orders"` (which it matches exactly). Same shape as `objects`.
        assert_eq!(quoting("SELECT * FROM Orders"), vec![vec![false]]);
        assert_eq!(quoting(r#"SELECT * FROM "Orders""#), vec![vec![true]]);
        assert_eq!(
            quoting(r#"SELECT * FROM "analytics".public.orders"#),
            vec![vec![true, false, false]]
        );
        assert_eq!(
            quoting("SELECT a FROM orders o JOIN \"customers\" c ON o.cid = c.id"),
            vec![vec![false], vec![true]]
        );
    }

    #[test]
    fn mysql_backtick_quoting_is_recorded() {
        // Any quote style the dialect writes marks the part quoted; the
        // review only ever asks "was it quoted".
        let r = sql_references("SELECT * FROM `Orders`", SqlDialect::Mysql).unwrap();
        assert_eq!(r.objects, vec![vec!["Orders"]]);
        assert_eq!(r.object_quoting, vec![vec![true]]);
    }

    #[test]
    fn same_spelling_with_different_quoting_are_distinct_objects() {
        // Postgres folds the unquoted factor to `orders` and reads the
        // quoted one as `Orders` — two different tables — so dedup must not
        // collapse them on spelling alone.
        let r = sql_references(r#"SELECT * FROM Orders, "Orders""#, D).unwrap();
        assert_eq!(r.objects, vec![vec!["Orders"], vec!["Orders"]]);
        assert_eq!(r.object_quoting, vec![vec![false], vec![true]]);
    }
}
