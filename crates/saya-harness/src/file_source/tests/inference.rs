use super::*;

#[test]
fn infers_column_types_from_the_sampled_rows() {
    let root = temp_root("infer");
    let source = root.join("in").join("types.csv");
    write_bytes(
        &source,
        b"n,d,b,dt,ts,txt,mixed,holes\n\
          1,1.5,true,2024-01-02,2024-01-02T03:04:05Z,hello,1,\n\
          2,2.5,false,2024-02-03,2024-03-04T05:06:07+00:30,world,x,\n",
    );
    let dest = root.join("dest");
    let staged = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("staging succeeds");
    assert_eq!(
        infer_of(&staged),
        [
            InferredType::Integer,
            InferredType::Decimal,
            InferredType::Boolean,
            InferredType::Date,
            InferredType::Timestamp,
            InferredType::Text,
            InferredType::Text,
            InferredType::Text,
        ]
    );
    assert_eq!(staged.preview.columns[7].null_count, 2);
    assert_eq!(staged.preview.sample_rows.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn leading_zero_numerics_and_non_iso_dates_stay_text() {
    let root = temp_root("leading");
    let source = root.join("in").join("leading.csv");
    write_bytes(&source, b"code,amount,when\n007,008.5,2024-1-5\n");
    let dest = root.join("dest");
    let staged = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: Some(b','),
            header: true,
        },
    )
    .expect("staging succeeds");
    assert_eq!(infer_of(&staged), [InferredType::Text; 3]);
    let _ = fs::remove_dir_all(root);
}
