use super::*;

#[test]
fn format_sql_breaks_before_keywords_and_preserves_casing() {
    let raw = "select 1 from t where a=1 limit 10";
    let formatted = format_sql(raw);
    assert_eq!(formatted, "select 1\nfrom t\nwhere a=1\nlimit 10");
}

#[test]
fn format_sql_collapses_whitespace_first() {
    let raw = "SELECT   1\n  FROM   t\n\n  WHERE   a = 1";
    let formatted = format_sql(raw);
    assert_eq!(formatted, "SELECT 1\nFROM t\nWHERE a = 1");
}

#[test]
fn format_sql_handles_multibyte_utf8() {
    let raw = "select 'café' from t";
    let formatted = format_sql(raw);
    assert_eq!(formatted, "select 'café'\nfrom t");
}

#[test]
fn format_sql_preserves_quoted_keywords_and_whitespace() {
    let raw = "SELECT 'a   from b', \"order by\" FROM t";
    let formatted = format_sql(raw);
    assert_eq!(formatted, "SELECT 'a   from b', \"order by\"\nFROM t");
}

#[test]
fn format_sql_preserves_mysql_and_postgres_quoted_forms() {
    let raw = "SELECT `from  table`, $tag$where from body$tag$ FROM t";
    let formatted = format_sql(raw);
    assert_eq!(
        formatted,
        "SELECT `from  table`, $tag$where from body$tag$\nFROM t"
    );
}

#[test]
fn format_sql_preserves_keywords_inside_comments() {
    let raw = "SELECT 1 -- from the cache\nFROM t /* where ignored */ WHERE id = 1";
    let formatted = format_sql(raw);
    assert_eq!(
        formatted,
        "SELECT 1 -- from the cache\nFROM t /* where ignored */\nWHERE id = 1"
    );
}

#[test]
fn sql_tool_call_bounded_sql_query_default() {
    let args = serde_json::json!({ "sql": "select 1" });
    let call = sql_tool_call("bounded_sql_query", &args).expect("should return SqlCall");
    assert_eq!(call.target, None);
    assert_eq!(call.sql, "select 1");
}

#[test]
fn sql_tool_call_bounded_sql_query_with_connection() {
    let args = serde_json::json!({ "sql": "select 1", "connection": "wh" });
    let call = sql_tool_call("bounded_sql_query", &args).expect("should return SqlCall");
    assert_eq!(call.target, Some("@wh".to_string()));
    assert_eq!(call.sql, "select 1");
}

#[test]
fn sql_tool_call_bounded_sql_query_all_no_connection() {
    let args = serde_json::json!({ "sql": "select 1" });
    let call = sql_tool_call("bounded_sql_query_all", &args).expect("should return SqlCall");
    assert_eq!(call.target, Some("all connected databases".to_string()));
    assert_eq!(call.sql, "select 1");
}

#[test]
fn sql_tool_call_schema_discovery_returns_none() {
    let args = serde_json::json!({ "connection": "wh" });
    assert!(sql_tool_call("schema_discovery", &args).is_none());
}
