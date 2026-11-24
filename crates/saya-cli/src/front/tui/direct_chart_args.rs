//! `/chart [kind] [path]`, parsed by the grammar the old TUI read.
//!
//! The grammar is one function because two readings of one argument string
//! would drift: a leading token that names a chart kind is the kind, and what
//! follows is the path; a leading token that is not a kind is the path, with
//! the kind left to the suggester. The old TUI's dispatcher owned it until X1
//! deleted the dispatcher; the only caller left is this surface's `/chart`
//! worker, so the parser moved here unchanged.

/// Splits `/chart`'s argument tail into an explicit kind and a path, either of
/// which may be absent.
pub(crate) fn parse_chart_args(args: &str) -> (Option<crate::chart::ChartKind>, Option<String>) {
    let mut tokens = args.split_whitespace();
    match tokens.next() {
        Some(tok) => match crate::chart::ChartKind::parse(tok) {
            Some(kind) => (Some(kind), tokens.next().map(str::to_string)),
            None => (None, Some(tok.to_string())),
        },
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_chart_args;

    /// A leading kind names the kind, and the next token is the path.
    #[test]
    fn a_leading_kind_is_the_kind_and_what_follows_is_the_path() {
        let (kind, path) = parse_chart_args("bar out.html");
        assert_eq!(kind, crate::chart::ChartKind::parse("bar"));
        assert_eq!(path.as_deref(), Some("out.html"), "the path is the tail");
    }

    /// A leading token that is not a kind is the path, and the suggester
    /// chooses the kind — `/chart out.html` writes a suggested chart.
    #[test]
    fn a_leading_path_leaves_the_kind_to_the_suggester() {
        let (kind, path) = parse_chart_args("out.html");
        assert_eq!(kind, None, "no kind was named");
        assert_eq!(path.as_deref(), Some("out.html"));
    }

    /// No argument at all names neither: the suggester picks the kind and the
    /// chart goes to a reserved temp file.
    #[test]
    fn an_empty_tail_names_nothing() {
        assert_eq!(parse_chart_args("   "), (None, None));
    }
}
