#[test]
fn malformed_chunks_and_secrets_do_not_surface_values() {
    let error = super::super::result::chunk_rows("secret").unwrap_err();
    assert!(!error.to_string().contains("secret"));
}
