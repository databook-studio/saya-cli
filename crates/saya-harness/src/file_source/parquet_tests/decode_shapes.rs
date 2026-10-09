use super::*;

#[test]
fn parquet_decode_cap_prevents_expansion() {
    let root = temp_root("cap");
    let source = flat_fixture(&root, "small.parquet");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let read = source_read(&source);
    let caps = ParquetCaps {
        max_rows: 2,
        ..ParquetCaps::default()
    };
    let error = parquet_stage::stage_read(read, &dest, caps).expect_err("cap refusal");
    assert!(
        matches!(error, StageError::ParquetTooManyRows { rows: 3, max: 2 }),
        "typed row-cap refusal, got {error:?}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_column_cap_is_enforced() {
    let root = temp_root("cols");
    let source = flat_fixture(&root, "three.parquet");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let read = source_read(&source);
    let caps = ParquetCaps {
        max_columns: 2,
        ..ParquetCaps::default()
    };
    let error = parquet_stage::stage_read(read, &dest, caps).expect_err("column cap refusal");
    assert!(
        matches!(
            error,
            StageError::ParquetTooManyColumns { columns: 3, max: 2 }
        ),
        "typed column-cap refusal, got {error:?}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_nested_types_fail_explicitly() {
    let root = temp_root("nested");
    let source = root.join("in").join("nested.parquet");
    write_parquet_fixture(&source, "SELECT {'a': 1, 'b': 'x'} AS s, [1, 2, 3] AS l");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let error = stage_parquet(&source, &dest).expect_err("nested refused");
    let StageError::ParquetNestedColumn { column, kind } = error else {
        panic!("expected a nested-column refusal, got {error:?}");
    };
    assert!(column == "s" || column == "l", "names the column: {column}");
    let upper = kind.to_ascii_uppercase();
    assert!(
        upper.contains("STRUCT(") || upper.contains('['),
        "shows the nested type: {kind}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}
