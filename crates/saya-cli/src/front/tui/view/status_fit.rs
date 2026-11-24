//! Fitting the status line into the columns a terminal gives it.
//!
//! The row's drop order, first dropped first: the meter, the extra database
//! count, the right side's hint, and the sharing caveat's detail. The mode chip,
//! the model and the database have no step at all — a narrow terminal is not a
//! reason to forget which model is answering or what it is talking to, and the
//! chip is the row's reason to exist. A row too narrow for even those overflows
//! rather than hides one.

use ratatui::style::Style;

use crate::interactive::tui::wrap::cell_width;

/// The join between the row's left segments.
pub(crate) const SEP: &str = " │ ";

/// The least air between the left group and the right-aligned one, so the two
/// never read as one run of words.
pub(crate) const GAP: usize = 2;

/// One step of the drop order, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// The context and cost meter: the row's newest and least urgent fact, so
    /// the first one a narrow row gives up.
    Meters,
    /// The extra database count — the database keeps its name, never leaves.
    Database,
    /// The right side's hint, before its state word.
    Hint,
    /// `⚠ data sharing on` reading `⚠ sharing`: the caveat stays, its detail
    /// does not.
    Sharing,
    /// The right side's state word.
    State,
}

/// The order the row gives way in.
pub(crate) const ORDER: [Step; 5] = [
    Step::Meters,
    Step::Database,
    Step::Hint,
    Step::Sharing,
    Step::State,
];

/// One segment of the row's left group: its words, the style it paints, and
/// the step that takes it away (`None`: no step does).
pub(crate) struct Segment {
    /// What the segment says now.
    pub(crate) text: String,
    /// The shorter form it takes instead of leaving the row.
    pub(crate) short: Option<String>,
    /// The style it paints — the token's form for the active scheme.
    pub(crate) paint: Style,
    /// The step that removes it, or `None` for a segment that never goes.
    pub(crate) step: Option<Step>,
}

impl Segment {
    /// A segment `step` takes away, with no shorter form owed to it.
    pub(crate) fn new(text: impl Into<String>, paint: Style, step: Option<Step>) -> Self {
        Self {
            text: text.into(),
            short: None,
            paint,
            step,
        }
    }

    /// The same segment, with the form it keeps when its step arrives.
    pub(crate) fn shortening(mut self, short: impl Into<String>) -> Self {
        self.short = Some(short.into());
        self
    }
}

/// The row's right side: what the surface is doing.
pub(crate) enum Right {
    /// A state's word and the hint beside it, which drops before the word.
    State {
        word: String,
        hint: Option<String>,
        paint: Style,
    },
    /// Nothing to say.
    Empty,
}

impl Right {
    /// The right side as one run of words, separators included.
    pub(crate) fn text(&self) -> String {
        match self {
            Self::State { word, hint, .. } => match hint {
                Some(hint) => format!("{word} · {hint}"),
                None => word.clone(),
            },
            Self::Empty => String::new(),
        }
    }

    /// The cells the right side occupies.
    pub(crate) fn width(&self) -> usize {
        cell_width(&self.text())
    }
}

/// The cells the left group occupies, separators included.
pub(crate) fn left_width(segments: &[Segment]) -> usize {
    let texts = segments
        .iter()
        .map(|segment| cell_width(&segment.text))
        .sum::<usize>();
    texts + segments.len().saturating_sub(1) * cell_width(SEP)
}

/// The cells the whole row wants, gap included.
pub(crate) fn row_width(segments: &[Segment], right: &Right) -> usize {
    let left = left_width(segments);
    match right.width() {
        0 => left,
        tail => left + GAP + tail,
    }
}

/// Fits the row into `width` cells by the drop order above.
///
/// A row that still does not fit once every step has been taken overflows: the
/// chip, the model and the database are the row's reason to exist, so it would
/// rather be too wide than quiet about them.
pub(crate) fn fit(segments: &mut Vec<Segment>, right: &mut Right, width: usize) {
    for step in ORDER {
        if row_width(segments, right) <= width {
            return;
        }
        apply(step, segments, right);
    }
}

/// Takes one step's worth of row: a whole segment, or the shorter form a
/// segment owes instead of leaving.
fn apply(step: Step, segments: &mut Vec<Segment>, right: &mut Right) {
    match step {
        // The meter is one fact too many on a short row, so it leaves whole:
        // there is no shorter way to say "40% of the window".
        Step::Meters => segments.retain(|segment| segment.step != Some(Step::Meters)),
        Step::Hint => {
            if let Right::State { hint, .. } = right {
                *hint = None;
            }
        }
        // The state word is the row's own to drop.
        Step::State => {
            if matches!(right, Right::State { .. }) {
                *right = Right::Empty;
            }
        }
        // The sharing caveat and the database keep a form of their own: they
        // give way in words, never in presence.
        Step::Sharing | Step::Database => shorten(segments, step),
    }
}

/// Puts the segment `step` takes away into its shorter form, if it has one.
fn shorten(segments: &mut [Segment], step: Step) {
    let found = segments
        .iter_mut()
        .find(|segment| segment.step == Some(step));
    if let Some(segment) = found
        && let Some(short) = segment.short.take()
    {
        segment.text = short;
    }
}
