//! Chart kinds and specifications.

/// Supported chart kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ChartKind {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
}

impl ChartKind {
    /// Parses a case-insensitive kind name; returns None if unrecognized.
    #[allow(dead_code)]
    pub(crate) fn parse(s: &str) -> Option<ChartKind> {
        match s.to_lowercase().as_str() {
            "bar" => Some(ChartKind::Bar),
            "line" => Some(ChartKind::Line),
            "area" => Some(ChartKind::Area),
            "pie" => Some(ChartKind::Pie),
            "doughnut" => Some(ChartKind::Doughnut),
            "scatter" => Some(ChartKind::Scatter),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(super) fn chartjs_type(self) -> &'static str {
        match self {
            ChartKind::Bar => "bar",
            ChartKind::Line | ChartKind::Area => "line",
            ChartKind::Pie => "pie",
            ChartKind::Doughnut => "doughnut",
            ChartKind::Scatter => "scatter",
        }
    }
}

/// Which columns to plot and how.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ChartSpec {
    pub(crate) kind: ChartKind,
    pub(crate) x: Option<String>, // label / x-axis column name; None => auto
    pub(crate) y: Vec<String>,    // value column name(s); empty => auto
    pub(crate) title: Option<String>,
}
