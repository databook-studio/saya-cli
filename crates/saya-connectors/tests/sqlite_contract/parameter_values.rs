use super::*;

// --- Native parameter binding (B1c) ---

/// A value carrying SQL syntax must reach the engine as a bound literal, not
/// as text that escapes the comparison and matches every row.
#[tokio::test]
async fn typed_values_bind_without_interpolation() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("params_injection.db");
    create_param_fixture(&path).await;
    let connector = open_param_connector(&path).await;

    let request = QueryRequest::with_params(
        "SELECT id FROM params_t WHERE label = :label".to_string(),
        10,
        vec![bound("label", ParamValue::String("' OR 1=1 --".to_owned()))],
    );
    let result = connector.execute(request).await.unwrap();
    assert_eq!(
        result.row_count, 0,
        "an interpolated value would have matched every row"
    );

    drop(connector);
    drop(temp_dir);
}

/// Every declared parameter type binds natively and survives the round trip.
#[tokio::test]
async fn every_parameter_type_round_trips_through_a_native_bind() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("params_types.db");
    create_param_fixture(&path).await;
    let connector = open_param_connector(&path).await;

    for (sql, name, value, expected) in [
        (
            "SELECT id FROM params_t WHERE label = :label",
            "label",
            ParamValue::String("beta".to_owned()),
            Value::from(2),
        ),
        (
            "SELECT id FROM params_t WHERE id = :id",
            "id",
            ParamValue::Integer(1),
            Value::from(1),
        ),
        (
            "SELECT id FROM params_t WHERE id = :id",
            "id",
            ParamValue::Boolean(true),
            Value::from(1),
        ),
        (
            "SELECT :flag AS flag",
            "flag",
            ParamValue::Boolean(false),
            Value::from(0),
        ),
        (
            "SELECT :amount AS amount",
            "amount",
            ParamValue::Decimal("12.5".to_owned()),
            Value::from("12.5"),
        ),
        (
            "SELECT id FROM params_t WHERE day = :day",
            "day",
            ParamValue::Date("2024-02-29".to_owned()),
            Value::from(1),
        ),
        (
            "SELECT id FROM params_t WHERE seen_at = :seen_at",
            "seen_at",
            ParamValue::Timestamp("2024-02-29T03:04:05Z".to_owned()),
            Value::from(1),
        ),
        (
            "SELECT :seen_at AS seen_at",
            "seen_at",
            ParamValue::Timestamp("2024-01-01T02:00:00+02:00".to_owned()),
            Value::from("2024-01-01T02:00:00+02:00"),
        ),
    ] {
        let request = QueryRequest::with_params(sql.to_string(), 10, vec![bound(name, value)]);
        let result = connector.execute(request).await.unwrap();
        assert_eq!(
            only_row(&result)[0],
            expected,
            "{sql} bound the wrong value"
        );
    }

    drop(connector);
    drop(temp_dir);
}

/// An explicit null binds as SQL NULL, which `IS NULL` sees and `COALESCE`
/// replaces — not as a string.
#[tokio::test]
async fn null_binds_as_sql_null() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("params_null.db");
    create_param_fixture(&path).await;
    let connector = open_param_connector(&path).await;

    for (sql, expected) in [
        ("SELECT :v IS NULL AS is_null", Value::from(1)),
        ("SELECT COALESCE(:v, 'absent') AS v", Value::from("absent")),
    ] {
        let request = QueryRequest::with_params(
            sql.to_string(),
            10,
            vec![bound("v", ParamValue::Null(ParamType::String))],
        );
        let result = connector.execute(request).await.unwrap();
        assert_eq!(only_row(&result)[0], expected, "{sql}");
    }

    drop(connector);
    drop(temp_dir);
}

/// A repeated `:name` re-binds the same value at every site.
#[tokio::test]
async fn repeated_placeholder_binds_its_value_at_every_site() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("params_repeated.db");
    create_param_fixture(&path).await;
    let connector = open_param_connector(&path).await;

    let request = QueryRequest::with_params(
        "SELECT :v AS a, :v AS b".to_string(),
        10,
        vec![bound("v", ParamValue::Integer(7))],
    );
    let result = connector.execute(request).await.unwrap();
    assert_eq!(only_row(&result), &vec![Value::from(7), Value::from(7)]);

    drop(connector);
    drop(temp_dir);
}
