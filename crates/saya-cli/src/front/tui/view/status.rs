//! The new TUI's status line: the one row that names the mode in force, what
//! the session can touch and what the surface is doing — as words first,
//! colour second.
//!
//! A pure function of the facts and a width. Nothing here reads the terminal,
//! the clock or the model: the caller hands over what it knows, and the row
//! answers with the spans to paint. Which segment gives way when the row does
//! not fit is [`super::status_fit`]'s rule; this module says what the row would
//! like to show.

use ratatui::text::{Line, Span};
use saya_agent::PermissionMode;

use super::status_fit::{GAP, Right, SEP, Segment, fit, left_width};
use super::status_segments::segments;
use crate::front::tui::theme::{Charset, Glyph, Scheme, Token, glyph, style};

/// What the surface is doing, as the row's right side says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ActivityState {
    /// Nothing runs: the row offers the help key.
    #[default]
    Idle,
    /// A turn has been running for `seconds`.
    Working { seconds: u64 },
    /// A message waits behind the running turn.
    Queued { count: usize },
}

impl ActivityState {
    /// The state's own word, which outlives its hint.
    fn word(self, charset: Charset) -> String {
        match self {
            Self::Idle => format!("{} help", glyph(Glyph::Ask, charset)),
            Self::Working { seconds } => {
                format!("{} working {seconds}s", glyph(Glyph::Bullet, charset))
            }
            Self::Queued { count } => format!("queued {count}"),
        }
    }

    /// The one hint beside the word, which is the first half of the right side
    /// to go.
    fn hint(self) -> Option<String> {
        match self {
            Self::Idle => None,
            Self::Working { .. } => Some("Esc stop".to_owned()),
            Self::Queued { .. } => Some("Ctrl+G drop".to_owned()),
        }
    }

    /// The token the state word paints.
    const fn token(self) -> Token {
        match self {
            Self::Idle => Token::Dim,
            Self::Working { .. } => Token::Ok,
            Self::Queued { .. } => Token::Info,
        }
    }
}

/// Everything the row says, as the facts the session holds.
#[derive(Debug, Clone)]
pub(crate) struct StatusFacts {
    /// The mode in force — the chip, which no width takes away.
    pub(crate) mode: PermissionMode,
    /// The session's databases, the primary first.
    pub(crate) databases: Vec<String>,
    /// The model's short id; empty paints no model segment.
    pub(crate) model: String,
    /// The operator-facing workspace root (`~/shop`), already abbreviated —
    /// the row never reads the environment to shorten it.
    pub(crate) workspace: Option<String>,
    /// Whether host commands run outside the sandbox.
    pub(crate) host_unsandboxed: bool,
    /// Whether query data leaves the machine.
    pub(crate) sharing: bool,
    /// The programs the deny list keeps out.
    pub(crate) deny: Vec<String>,
    /// The provider's last context report, if it made one.
    pub(crate) context_percent: Option<u8>,
    /// How many background jobs the session holds.
    pub(crate) jobs: usize,
    /// What the surface is doing.
    pub(crate) activity: ActivityState,
    /// The surface's own last words, which replace the right side while they
    /// live.
    pub(crate) toast: Option<String>,
    /// Which token column paints the row.
    pub(crate) scheme: Scheme,
    /// Which glyph set the row draws with.
    pub(crate) charset: Charset,
}

impl Default for StatusFacts {
    fn default() -> Self {
        Self {
            mode: PermissionMode::default(),
            databases: Vec::new(),
            model: String::new(),
            workspace: None,
            host_unsandboxed: false,
            sharing: false,
            deny: Vec::new(),
            context_percent: None,
            jobs: 0,
            activity: ActivityState::default(),
            toast: None,
            scheme: Scheme::Dark,
            charset: Charset::Utf8,
        }
    }
}

/// The row, fitted to `width` cells.
pub(crate) fn status_line(facts: &StatusFacts, width: usize) -> Line<'static> {
    let mut segments = segments(facts);
    let mut right = right(facts);
    fit(&mut segments, &mut right, width);
    render(facts, &segments, &right, width)
}

/// Paints the fitted row: the left group, the air that right-aligns the rest,
/// and the right side.
fn render(facts: &StatusFacts, segments: &[Segment], right: &Right, width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(SEP, style(Token::Dim, facts.scheme)));
        }
        spans.push(Span::styled(segment.text.clone(), segment.paint));
    }
    if let Some(tail) = tail_spans(right, facts.scheme) {
        let air = width.saturating_sub(left_width(segments) + right.width());
        spans.push(Span::raw(" ".repeat(GAP.max(air))));
        spans.extend(tail);
    }
    Line::from(spans).style(style(Token::Surface, facts.scheme))
}

/// The right side's spans, or nothing when it has nothing to say.
fn tail_spans(right: &Right, scheme: Scheme) -> Option<Vec<Span<'static>>> {
    match right {
        Right::State { word, hint, paint } => {
            let mut spans = vec![Span::styled(word.clone(), *paint)];
            if let Some(hint) = hint {
                spans.push(Span::styled(
                    format!(" · {hint}"),
                    style(Token::Dim, scheme),
                ));
            }
            Some(spans)
        }
        Right::Toast { text, paint } => Some(vec![Span::styled(text.clone(), *paint)]),
        Right::Empty => None,
    }
}

/// The row's right side: a toast while one lives, otherwise the state.
fn right(facts: &StatusFacts) -> Right {
    if let Some(toast) = &facts.toast {
        return Right::Toast {
            text: toast.clone(),
            paint: style(Token::Info, facts.scheme),
        };
    }
    Right::State {
        word: facts.activity.word(facts.charset),
        hint: facts.activity.hint(),
        paint: style(facts.activity.token(), facts.scheme),
    }
}
