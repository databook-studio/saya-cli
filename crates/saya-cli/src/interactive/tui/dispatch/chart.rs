//! Parses "/chart [type] [path]": a leading known kind is consumed;
//! anything left is the output path.

/// Parses "/chart [type] [path]": a leading known kind is consumed; anything
/// left is the output path.
pub(super) fn parse_chart_args(args: &str) -> (Option<crate::chart::ChartKind>, Option<String>) {
    let mut tokens = args.split_whitespace();
    match tokens.next() {
        Some(tok) => match crate::chart::ChartKind::parse(tok) {
            Some(kind) => (Some(kind), tokens.next().map(str::to_string)),
            None => (None, Some(tok.to_string())),
        },
        None => (None, None),
    }
}
