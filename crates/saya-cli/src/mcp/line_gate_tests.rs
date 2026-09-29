//! Unit tests for the bounded line gate (invariant 1): accepted lines are
//! delivered whole, oversized lines are refused exactly once at their
//! newline, and a trailing fragment at EOF is dropped without a refusal. The
//! refusal frame itself is pinned over the wire in `tests/mcp_stdio.rs`.

use super::LineGate;

/// Drives `absorb` over the given byte slices and collects outcomes.
fn run(gate: &mut LineGate, chunks: &[&[u8]]) -> (Vec<String>, usize) {
    let mut delivered = Vec::new();
    let mut refused = 0;
    for chunk in chunks {
        gate.absorb(
            chunk,
            &mut |line| delivered.push(String::from_utf8_lossy(line).into_owned()),
            &mut || refused += 1,
        );
    }
    (delivered, refused)
}

#[test]
fn complete_lines_within_the_cap_are_delivered() {
    let mut gate = LineGate::new(1024);
    let (delivered, refused) = run(&mut gate, &[b"alpha\nbeta\ngamma\n"]);
    assert_eq!(delivered, ["alpha", "beta", "gamma"]);
    assert_eq!(refused, 0);
}

#[test]
fn a_line_split_across_chunks_is_assembled_before_delivery() {
    let mut gate = LineGate::new(1024);
    let (delivered, refused) = run(&mut gate, &[b"he", b"llo ", b"world\nnext\n"]);
    assert_eq!(delivered, ["hello world", "next"]);
    assert_eq!(refused, 0);
}

#[test]
fn an_oversized_line_is_refused_once_at_its_newline() {
    let cap = 64;
    let mut gate = LineGate::new(cap);
    let oversized = vec![b'x'; cap + 1];
    let mut chunks: Vec<&[u8]> = Vec::new();
    // Split the oversized line and a following short line into small pieces,
    // so the discard has to survive many chunks and still end once.
    let tail: Vec<u8> = b"\nok\n".to_vec();
    let mut bytes = oversized;
    bytes.extend_from_slice(&tail);
    for piece in bytes.chunks(7) {
        chunks.push(piece);
    }
    let (delivered, refused) = run(&mut gate, &chunks);
    assert_eq!(refused, 1, "one line, one refusal, exactly at its newline");
    assert_eq!(
        delivered,
        ["ok"],
        "the next line is served after the refusal"
    );
}

#[test]
fn a_line_at_exactly_the_cap_is_accepted() {
    let mut gate = LineGate::new(8);
    let (delivered, refused) = run(&mut gate, &[b"12345678\nmore\n"]);
    assert_eq!(delivered, ["12345678", "more"]);
    assert_eq!(refused, 0);
}

#[test]
fn an_unterminated_fragment_at_eof_is_dropped_without_a_refusal() {
    let mut gate = LineGate::new(8);
    let (delivered, refused) = run(&mut gate, &[b"ok\nno newline at eof"]);
    gate.finish();
    assert_eq!(delivered, ["ok"]);
    assert_eq!(refused, 0, "no complete oversized request, no answer");
}
