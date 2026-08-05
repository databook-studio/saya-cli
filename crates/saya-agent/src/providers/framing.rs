pub(super) fn whitespace(value: &[u8]) -> bool {
    value.iter().all(u8::is_ascii_whitespace)
}
