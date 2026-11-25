// The terminal side of the pty conversation.
//
// ConPTY is created with the inherit-cursor flag, so at startup it asks the
// client where the cursor is (`ESC [ 6 n`) and holds every byte of child
// output until the client answers (`ESC [ row ; col R`). A real terminal
// answers; a harness that only reads leaves the child blocked and the screen
// blank. This is the answering side, the same on every platform.

/// ConPTY's "where is the cursor?" query.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
/// The "are you OK?" status query.
const STATUS_QUERY: &[u8] = b"\x1b[5n";
/// Primary device attributes, with and without the explicit zero.
const PRIMARY_DA: [&[u8]; 2] = [b"\x1b[c", b"\x1b[0c"];
/// Secondary device attributes.
const SECONDARY_DA: &[u8] = b"\x1b[>c";

/// Answers the terminal queries in a stream, one read at a time.
///
/// `pending` holds a trailing prefix of a query split across reads, so a
/// sequence delivered in two chunks is still answered exactly once.
#[derive(Default)]
struct Answers {
    pending: Vec<u8>,
}

impl Answers {
    /// Appends `bytes` and returns the replies a terminal would send. `cursor`
    /// is the 1-based `(row, col)` a cursor-position query should report.
    fn feed(&mut self, bytes: &[u8], cursor: (u16, u16)) -> Vec<u8> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        let mut at = 0;
        while at < self.pending.len() {
            let rest = &self.pending[at..];
            if rest.starts_with(CURSOR_QUERY) {
                out.extend_from_slice(format!("\x1b[{};{}R", cursor.0, cursor.1).as_bytes());
                at += CURSOR_QUERY.len();
            } else if rest.starts_with(STATUS_QUERY) {
                out.extend_from_slice(b"\x1b[0n");
                at += STATUS_QUERY.len();
            } else if let Some(query) = PRIMARY_DA.iter().find(|q| rest.starts_with(q)) {
                out.extend_from_slice(b"\x1b[?1;0c");
                at += query.len();
            } else if rest.starts_with(SECONDARY_DA) {
                out.extend_from_slice(b"\x1b[>0;0;0c");
                at += SECONDARY_DA.len();
            } else if is_partial_query(rest) {
                // A query split across reads: keep the tail for the next one.
                break;
            } else {
                at += 1;
            }
        }
        self.pending.drain(..at);
        out
    }
}

/// True when `rest` is a prefix of a query and could still become one: the
/// tail is held rather than treated as output.
fn is_partial_query(rest: &[u8]) -> bool {
    const QUERIES: [&[u8]; 5] = [
        CURSOR_QUERY,
        STATUS_QUERY,
        PRIMARY_DA[0],
        PRIMARY_DA[1],
        SECONDARY_DA,
    ];
    QUERIES
        .iter()
        .any(|query| query.len() > rest.len() && query.starts_with(rest))
}

#[test]
fn a_cursor_query_is_answered_with_the_current_cursor() {
    assert_eq!(
        Answers::default().feed(b"\x1b[6n", (1, 1)),
        b"\x1b[1;1R".to_vec()
    );
    assert_eq!(
        Answers::default().feed(b"\x1b[6n", (4, 12)),
        b"\x1b[4;12R".to_vec()
    );
}

#[test]
fn device_and_status_queries_get_the_replies_a_terminal_sends() {
    assert_eq!(
        Answers::default().feed(b"\x1b[c", (1, 1)),
        b"\x1b[?1;0c".to_vec()
    );
    assert_eq!(
        Answers::default().feed(b"\x1b[0c", (1, 1)),
        b"\x1b[?1;0c".to_vec()
    );
    assert_eq!(
        Answers::default().feed(b"\x1b[>c", (1, 1)),
        b"\x1b[>0;0;0c".to_vec()
    );
    assert_eq!(
        Answers::default().feed(b"\x1b[5n", (1, 1)),
        b"\x1b[0n".to_vec()
    );
}

#[test]
fn ordinary_output_is_not_answered() {
    assert_eq!(
        Answers::default().feed(b"hello \x1b[31mred\x1b[0m \xe2\x94\x82", (1, 1)),
        Vec::new()
    );
}

#[test]
fn a_query_split_across_reads_is_answered_once() {
    let mut answers = Answers::default();
    assert_eq!(answers.feed(b"\x1b[", (1, 1)), Vec::new());
    assert_eq!(answers.feed(b"6", (1, 1)), Vec::new());
    assert_eq!(answers.feed(b"n", (1, 1)), b"\x1b[1;1R".to_vec());
}

#[test]
fn several_queries_in_one_read_are_all_answered() {
    assert_eq!(
        Answers::default().feed(b"\x1b[6n\x1b[c", (1, 1)),
        b"\x1b[1;1R\x1b[?1;0c".to_vec()
    );
}
