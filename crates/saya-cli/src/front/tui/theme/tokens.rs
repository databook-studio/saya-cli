//! Semantic tokens (`02` §12): colour named by meaning, with a light, a dark
//! and a no-colour form each.

use ratatui::style::{Color, Modifier, Style};

use crate::interactive::tui::ui::theme::hue;

/// The bypass chip's word (`02` §4, §12; `01` §4): the chip is an inverse `bad`
/// run whose word never drops, and with colour gone it is the whole signal.
pub(crate) const BYPASS_CHIP: &str = "! BYPASS";

/// A token names meaning, never a hue (`02` §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Token {
    /// Prose and every ordinary word.
    Fg,
    /// Footers, captions, NULLs.
    Dim,
    /// The user's own message, under `❯`.
    User,
    /// Focus, titles, the accept-edits mode chip.
    Accent,
    /// The plan chip and links.
    Info,
    /// `✓` and a diff's `+`.
    Ok,
    /// `✗`, a diff's `−`, and the bypass chip.
    Bad,
    /// `⚠` caveats and `⏸`.
    Warn,
    /// Inline code and SQL.
    Code,
    /// The status line's and a card's ground.
    Surface,
    /// The cursor row.
    Focus,
}

impl Token {
    /// Every token, in the order `02` §12 lists them.
    pub(crate) const ALL: [Self; 11] = [
        Self::Fg,
        Self::Dim,
        Self::User,
        Self::Accent,
        Self::Info,
        Self::Ok,
        Self::Bad,
        Self::Warn,
        Self::Code,
        Self::Surface,
        Self::Focus,
    ];

    /// The token's name in the spec: what a snapshot, a comment or a
    /// design review says.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Fg => "fg",
            Self::Dim => "dim",
            Self::User => "user",
            Self::Accent => "accent",
            Self::Info => "info",
            Self::Ok => "ok",
            Self::Bad => "bad",
            Self::Warn => "warn",
            Self::Code => "code",
            Self::Surface => "surface",
            Self::Focus => "focus",
        }
    }
}

/// Which column of `02` §12 a form comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scheme {
    Light,
    Dark,
    NoColour,
}

/// One resolved form of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Form {
    /// The foreground hue; `None` when the form paints no colour at all.
    pub(crate) fg: Option<Color>,
    /// The background hue (`surface`); `None` for everything else.
    pub(crate) bg: Option<Color>,
    /// The weight and decoration that survive colour going away.
    pub(crate) modifiers: Modifier,
    /// The marker text a renderer prints beside the run where colour would
    /// otherwise be the only difference (`02` §12 rule 1).
    pub(crate) marker: Option<&'static str>,
}

impl Form {
    /// The style an unstyled run already gets: the baseline every form is
    /// measured against.
    pub(crate) const fn plain() -> Self {
        Self {
            fg: None,
            bg: None,
            modifiers: Modifier::empty(),
            marker: None,
        }
    }

    /// The ratatui style this form paints.
    pub(crate) fn style(self) -> Style {
        let mut style = Style::default().add_modifier(self.modifiers);
        if let Some(fg) = self.fg {
            style = style.fg(fg);
        }
        if let Some(bg) = self.bg {
            style = style.bg(bg);
        }
        style
    }
}

/// Resolves a token to one of its three forms.
pub(crate) fn form(token: Token, scheme: Scheme) -> Form {
    match scheme {
        Scheme::Dark => coloured(token, true),
        Scheme::Light => coloured(token, false),
        Scheme::NoColour => uncoloured(token),
    }
}

/// The light and dark columns of `02` §12: the same meaning, the same weight,
/// two hues. The hues themselves live in `palette` so a hue has one definition.
fn coloured(token: Token, dark: bool) -> Form {
    let pick = |dark_colour: Color, light_colour: Color| {
        if dark { dark_colour } else { light_colour }
    };
    let (hue, modifiers) = match token {
        // The terminal's own text on the terminal's own ground: the one colour
        // that can never be a wrong guess about the background.
        Token::Fg => (pick(Color::Reset, hue::FOREGROUND_LIGHT), Modifier::empty()),
        Token::User => (pick(Color::Reset, hue::FOREGROUND_LIGHT), Modifier::BOLD),
        Token::Dim => (pick(hue::DIM_DARK, hue::DIM_LIGHT), Modifier::empty()),
        Token::Accent => (pick(hue::ACCENT_DARK, hue::ACCENT_LIGHT), Modifier::BOLD),
        Token::Info => (pick(hue::INFO_DARK, hue::INFO_LIGHT), Modifier::empty()),
        Token::Ok => (pick(hue::OK_DARK, hue::OK_LIGHT), Modifier::empty()),
        Token::Bad => (pick(hue::BAD_DARK, hue::BAD_LIGHT), Modifier::empty()),
        Token::Warn => (pick(hue::WARN_DARK, hue::WARN_LIGHT), Modifier::empty()),
        Token::Code => (pick(hue::CODE_DARK, hue::CODE_LIGHT), Modifier::empty()),
        Token::Surface => (
            pick(hue::SURFACE_DARK, hue::SURFACE_LIGHT),
            Modifier::empty(),
        ),
        Token::Focus => (
            pick(hue::ACCENT_DARK, hue::ACCENT_LIGHT),
            Modifier::BOLD | Modifier::UNDERLINED,
        ),
    };
    if token == Token::Surface {
        // A ground paints a background; a foreground here would tint the very
        // cells it sits behind.
        Form {
            fg: None,
            bg: Some(hue),
            modifiers,
            marker: None,
        }
    } else {
        Form {
            fg: Some(hue),
            bg: None,
            modifiers,
            marker: None,
        }
    }
}

/// The no-colour column of `02` §12: what is left when every hue is gone.
///
/// `02` §12 rule 1 pairs every token with a glyph or a word in the same run, so
/// the tokens the table calls "plain" carry that pairing as a [`Form::marker`]
/// instead of leaving it to a renderer's memory.
fn uncoloured(token: Token) -> Form {
    let (modifiers, marker) = match token {
        // Prose *is* plain text without colour: the one form with no carrier,
        // because inventing one would dress prose up as something it is not.
        Token::Fg => (Modifier::empty(), None),
        Token::Dim => (Modifier::DIM, None),
        Token::User => (Modifier::BOLD, None),
        Token::Accent => (Modifier::BOLD, None),
        Token::Info => (Modifier::empty(), Some("info")),
        // The same word `02` §11's fallback prints: `[ok]`.
        Token::Ok => (Modifier::empty(), Some("ok")),
        // `!` is the word `01` §5.5 pairs with a refusal, and the chip's `!`
        // already means bypass everywhere else.
        Token::Bad => (Modifier::empty(), Some("!")),
        Token::Warn => (Modifier::empty(), Some("warn")),
        Token::Code => (Modifier::empty(), Some("code")),
        // The status line keeps its ` │ ` separators (`02` §12), so a separator
        // is the surface's carrier once no tint is painted.
        Token::Surface => (Modifier::empty(), Some("|")),
        Token::Focus => (Modifier::UNDERLINED, None),
    };
    Form {
        fg: None,
        bg: None,
        modifiers,
        marker,
    }
}

/// The style a token paints: the form, ready for a ratatui call site.
pub(crate) fn style(token: Token, scheme: Scheme) -> Style {
    form(token, scheme).style()
}

/// The bypass chip: inverse `bad`, and the word that never drops.
///
/// `01` §4 makes the chip a banner, so its word is not decoration for the
/// colour: with the hue gone, reverse video and `! BYPASS` are the whole chip.
pub(crate) fn bypass_chip(scheme: Scheme) -> Form {
    let bad = form(Token::Bad, scheme);
    Form {
        fg: bad.fg,
        bg: None,
        modifiers: bad.modifiers | Modifier::REVERSED,
        marker: Some(BYPASS_CHIP),
    }
}
