//! Fitting the status line into the columns a terminal gives it.
//!
//! The row's drop order, first dropped first: the workspace binding, the
//! context meter, the model id, the deny list, the extra database count, the
//! job count, the right side's hint, the sharing caveat's detail, and last the
//! right side's state word. The mode chip and the two caveats have no step at
//! all — nothing here can take them away, which is the point: a narrow
//! terminal stops being a reason to hide what the session can touch.

use ratatui::style::Style;

use crate::interactive::tui::wrap::{cell_width, truncate_cells};

/// The join between the row's left segments.
pub(crate) const SEP: &str = " │ ";

/// The least air between the left group and the right-aligned one, so the two
/// never read as one run of words.
pub(crate) const GAP: usize = 2;

/// One step of the drop order, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// The workspace binding: the session's own directory, which the operator
    /// already knows.
    Workspace,
    /// The context meter.
    Context,
    /// The model id.
    Model,
    /// The deny list.
    Deny,
    /// The extra database count — the database keeps a form, never leaves.
    Database,
    /// The background-job count.
    Jobs,
    /// The right side's hint, before its state word.
    Hint,
    /// `⚠ data sharing on` reading `⚠ sharing`: the caveat stays, its detail
    /// does not.
    Sharing,
    /// The right side's state word.
    State,
}

/// The order the row gives way in.
pub(crate) const ORDER: [Step; 9] = [
    Step::Workspace,
    Step::Context,
    Step::Model,
    Step::Deny,
    Step::Database,
    Step::Jobs,
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

/// The row's right side: what the surface is doing, or the one thing it is
/// saying.
pub(crate) enum Right {
    /// A state's word and the hint beside it, which drops before the word.
    State {
        word: String,
        hint: Option<String>,
        paint: Style,
    },
    /// A toast: the surface's own words, kept whole while they live.
    Toast { text: String, paint: Style },
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
            Self::Toast { text, .. } => text.clone(),
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
/// chip and the caveats are the row's reason to exist, so it would rather be
/// too wide than quiet about them.
pub(crate) fn fit(segments: &mut Vec<Segment>, right: &mut Right, width: usize) {
    for step in ORDER {
        if row_width(segments, right) <= width {
            return;
        }
        apply(step, segments, right);
    }
    cut_right_side(segments, right, width);
}

/// Takes one step's worth of row: a whole segment, or the shorter form a
/// segment owes instead of leaving.
fn apply(step: Step, segments: &mut Vec<Segment>, right: &mut Right) {
    match step {
        Step::Hint => {
            if let Right::State { hint, .. } = right {
                *hint = None;
            }
        }
        // The state word is the row's own to drop; a toast is the surface's
        // answer to the operator and is cut, not dropped.
        Step::State => {
            if matches!(right, Right::State { .. }) {
                *right = Right::Empty;
            }
        }
        // The sharing caveat and the database keep a form of their own: they
        // give way in words, never in presence.
        Step::Sharing | Step::Database => shorten(segments, step),
        _ => remove(segments, step),
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

/// Drops the segment `step` takes away.
fn remove(segments: &mut Vec<Segment>, step: Step) {
    if let Some(index) = segments
        .iter()
        .position(|segment| segment.step == Some(step))
    {
        segments.remove(index);
    }
}

/// The last resort: the surface's own toast, cut to the cells left over. A
/// caveat is never cut — half a caveat is worse than a row that overflows.
fn cut_right_side(segments: &[Segment], right: &mut Right, width: usize) {
    let (text, paint) = match right {
        Right::Toast { text, paint } => (text.clone(), *paint),
        _ => return,
    };
    let room = width.saturating_sub(left_width(segments) + GAP);
    if cell_width(&text) <= room {
        return;
    }
    let cut = truncate_cells(&text, room);
    *right = if cut.is_empty() {
        Right::Empty
    } else {
        Right::Toast { text: cut, paint }
    };
}
