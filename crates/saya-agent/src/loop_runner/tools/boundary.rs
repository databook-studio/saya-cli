/// Largest byte index `<= idx` that falls on a UTF-8 character boundary, so a
/// truncation slice never splits a multi-byte sequence. (`str::floor_char_boundary`
/// would do this but is stable only since 1.91, above the 1.88 MSRV.)
pub(in crate::loop_runner) fn floor_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        idx = text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}
