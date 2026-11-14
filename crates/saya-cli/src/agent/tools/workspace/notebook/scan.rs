//! The byte-span scanner the notebook arm needs: where the top-level `cells`
//! array is, and where each of its elements is.
//!
//! Raw spans rather than a parsed document, because `notebook_edit` splices
//! the file's own bytes: every cell it does not touch stays byte-for-byte what
//! it was, which `serde_json`'s `Value` cannot promise (its maps are unordered,
//! so re-serializing a document rewrites all of it). The scanner is the small
//! part of JSON parsing this needs — strings, nesting, and whitespace — and it
//! only ever *locates*: what it finds is parsed by `serde_json` afterwards, so
//! a value it mis-locates cannot be mistaken for a valid one.

use std::ops::Range;

/// A notebook's top-level `cells` array: the byte range between its brackets,
/// and the byte range of each element.
#[derive(Debug)]
pub(in crate::agent::tools) struct Cells {
    /// The bytes between `[` and `]` — where an insert into an empty array
    /// goes.
    pub(in crate::agent::tools) inner: Range<usize>,
    /// Each element's own bytes, in order.
    pub(in crate::agent::tools) elements: Vec<Range<usize>>,
}

/// Locates the top-level `cells` array of `bytes`.
///
/// `None` unless the bytes are a JSON object carrying both a `cells` array and
/// a numeric `nbformat` — the two keys nbformat itself requires. So a text
/// file named `.ipynb`, a truncated read, or any other JSON keeps the ordinary
/// text path rather than being served as a notebook it is not.
pub(in crate::agent::tools) fn scan(bytes: &[u8]) -> Option<Cells> {
    let mut cursor = skip_ws(bytes, 0);
    if bytes.get(cursor) != Some(&b'{') {
        return None;
    }
    cursor += 1;
    let mut cells = None;
    let mut nbformat = false;
    loop {
        cursor = skip_ws(bytes, cursor);
        if bytes.get(cursor) == Some(&b'}') {
            break;
        }
        if bytes.get(cursor) != Some(&b'"') {
            return None;
        }
        let key_end = string_end(bytes, cursor)?;
        let key = &bytes[cursor + 1..key_end - 1];
        cursor = skip_ws(bytes, key_end);
        if bytes.get(cursor) != Some(&b':') {
            return None;
        }
        cursor = skip_ws(bytes, cursor + 1);
        if key == b"cells" {
            cells = Some(array(bytes, cursor)?);
        } else if key == b"nbformat" {
            nbformat = is_number(bytes, cursor);
        }
        cursor = skip_ws(bytes, value_end(bytes, cursor)?);
        if bytes.get(cursor) == Some(&b',') {
            cursor += 1;
        }
    }
    (nbformat && cells.is_some()).then(|| cells.unwrap())
}

/// The elements of the array at `start`, which must be a `[`.
fn array(bytes: &[u8], start: usize) -> Option<Cells> {
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut cursor = skip_ws(bytes, start + 1);
    let mut elements = Vec::new();
    if bytes.get(cursor) == Some(&b']') {
        return Some(Cells {
            inner: start + 1..cursor,
            elements,
        });
    }
    loop {
        let begin = cursor;
        let end = value_end(bytes, begin)?;
        elements.push(begin..end);
        cursor = skip_ws(bytes, end);
        match bytes.get(cursor)? {
            b',' => cursor = skip_ws(bytes, cursor + 1),
            b']' => {
                return Some(Cells {
                    inner: start + 1..cursor,
                    elements,
                });
            }
            _ => return None,
        }
    }
}

/// The byte after the JSON value starting at `start`.
fn value_end(bytes: &[u8], start: usize) -> Option<usize> {
    let first = *bytes.get(start)?;
    if first == b'"' {
        return string_end(bytes, start);
    }
    if first == b'{' || first == b'[' {
        let (open, close) = if first == b'{' {
            (b'{', b'}')
        } else {
            (b'[', b']')
        };
        let mut depth = 0usize;
        let mut cursor = start;
        while let Some(byte) = bytes.get(cursor) {
            if *byte == b'"' {
                cursor = string_end(bytes, cursor)?;
                continue;
            }
            if *byte == open {
                depth += 1;
            } else if *byte == close {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor + 1);
                }
            }
            cursor += 1;
        }
        return None;
    }
    let mut cursor = start;
    while let Some(byte) = bytes.get(cursor) {
        if matches!(*byte, b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r') {
            break;
        }
        cursor += 1;
    }
    (cursor > start).then_some(cursor)
}

/// Whether the value at `start` is a JSON number, which is what `nbformat` is.
fn is_number(bytes: &[u8], start: usize) -> bool {
    value_end(bytes, start).is_some_and(|end| {
        bytes[start..end]
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'-' | b'+' | b'.' | b'e' | b'E'))
    })
}

/// The byte after the string starting at `start`, which must be a `"`.
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    while let Some(byte) = bytes.get(cursor) {
        match *byte {
            b'\\' => cursor += 2,
            b'"' => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    None
}

fn skip_ws(bytes: &[u8], mut cursor: usize) -> usize {
    while matches!(bytes.get(cursor), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        cursor += 1;
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::scan;

    const NOTEBOOK: &[u8] = br#"{
  "cells": [ {"a": [1, 2]}, {}, "text" ],
  "nbformat": 4
}"#;

    #[test]
    fn finds_the_cells_array_and_each_element() {
        let cells = scan(NOTEBOOK).expect("a notebook with a cells array must scan");
        assert_eq!(cells.elements.len(), 3);
        assert_eq!(&NOTEBOOK[cells.elements[0].clone()], br#"{"a": [1, 2]}"#);
        assert_eq!(&NOTEBOOK[cells.elements[1].clone()], b"{}");
        assert_eq!(&NOTEBOOK[cells.elements[2].clone()], b"\"text\"");
    }

    #[test]
    fn an_empty_cells_array_scans_to_no_elements() {
        let bytes = br#"{"cells": [], "nbformat": 4}"#;
        let cells = scan(bytes).expect("an empty notebook still scans");
        assert!(cells.elements.is_empty());
        assert_eq!(&bytes[cells.inner], b"");
    }

    #[test]
    fn braces_and_brackets_inside_strings_do_not_end_the_array() {
        let bytes = br#"{"cells": ["}{]"], "nbformat": 4}"#;
        let cells = scan(bytes).expect("a string holding brackets must not fool the scan");
        assert_eq!(cells.elements.len(), 1);
        assert_eq!(&bytes[cells.elements[0].clone()], br#""}{]""#);
    }

    #[test]
    fn an_escaped_quote_inside_a_string_does_not_end_it() {
        let bytes = br#"{"cells": ["a\"b"], "nbformat": 4}"#;
        let cells = scan(bytes).expect("an escaped quote must not fool the scan");
        assert_eq!(cells.elements.len(), 1);
        assert_eq!(&bytes[cells.elements[0].clone()], br#""a\"b""#);
    }

    #[test]
    fn json_without_both_notebook_keys_does_not_scan() {
        for bytes in [
            &br#"{"cells": []}"#[..],
            &br#"{"nbformat": 4}"#[..],
            &br#"{"cells": [], "nbformat": "4"}"#[..],
            &br#"{"hello": "world"}"#[..],
            &br#"[{"cells": [], "nbformat": 4}]"#[..],
            &br#"{"cells": [}"#[..],
            &br#"{"cells": [], "nbformat": 4"#[..],
        ] {
            assert!(
                scan(bytes).is_none(),
                "{} must not scan as a notebook",
                String::from_utf8_lossy(bytes)
            );
        }
    }
}
