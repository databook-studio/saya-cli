//! The glyph set of `02` §11 and its ASCII fallback.
//!
//! Every glyph the new TUI draws is named here, once, with the ASCII form that
//! replaces it under `--ascii` (or a non-UTF-8 locale). A caller never writes a
//! glyph or its fallback as a literal: it asks [`glyph`] for the one the active
//! [`Charset`] calls for.

/// One glyph of the UTF-8 set of `02` §11.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Glyph {
    /// The user's turn: `❯`.
    Prompt,
    /// Finished / passing: `✓`.
    Check,
    /// Failed / refused: `✗`.
    Cross,
    /// Denied, read-only: `⊘`.
    Denied,
    /// A question or help: `?`.
    Ask,
    /// A session open in this process: `●`.
    Bullet,
    /// Paused: `⏸`.
    Paused,
    /// Stopped / cancelled: `■`.
    Stop,
    /// An edit: `✎`.
    Edit,
    /// A hint or note: `◆`.
    Hint,
    /// A box's top-left corner: `╭`.
    TopLeft,
    /// A box's top-right corner: `╮`.
    TopRight,
    /// A box's bottom-left corner: `╰`.
    BottomLeft,
    /// A box's bottom-right corner: `╯`.
    BottomRight,
    /// A horizontal rule: `─`.
    Rule,
    /// A vertical edge: `│`.
    Edge,
    /// A collapsed row that opens: `▸`.
    Selected,
    /// An open group that folds: `▾`.
    Collapsed,
}

impl Glyph {
    /// Every glyph of `02` §11: each one has an ASCII form, and there are none
    /// outside that table.
    pub(crate) const ALL: [Self; 18] = [
        Self::Prompt,
        Self::Check,
        Self::Cross,
        Self::Denied,
        Self::Ask,
        Self::Bullet,
        Self::Paused,
        Self::Stop,
        Self::Edit,
        Self::Hint,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::Rule,
        Self::Edge,
        Self::Selected,
        Self::Collapsed,
    ];

    /// The glyph's name, for docs, tests and diagnostics.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Check => "check",
            Self::Cross => "cross",
            Self::Denied => "denied",
            Self::Ask => "ask",
            Self::Bullet => "bullet",
            Self::Paused => "paused",
            Self::Stop => "stop",
            Self::Edit => "edit",
            Self::Hint => "hint",
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
            Self::Rule => "rule",
            Self::Edge => "edge",
            Self::Selected => "selected",
            Self::Collapsed => "collapsed",
        }
    }

    /// The UTF-8 form: what the screens of `02` draw.
    pub(crate) fn utf8(self) -> &'static str {
        match self {
            Self::Prompt => "❯",
            Self::Check => "✓",
            Self::Cross => "✗",
            Self::Denied => "⊘",
            Self::Ask => "?",
            Self::Bullet => "●",
            Self::Paused => "⏸",
            Self::Stop => "■",
            Self::Edit => "✎",
            Self::Hint => "◆",
            Self::TopLeft => "╭",
            Self::TopRight => "╮",
            Self::BottomLeft => "╰",
            Self::BottomRight => "╯",
            Self::Rule => "─",
            Self::Edge => "│",
            Self::Selected => "▸",
            Self::Collapsed => "▾",
        }
    }

    /// The ASCII form from `02` §11: seven-bit, so it survives a terminal that
    /// cannot draw the UTF-8 set.
    pub(crate) fn ascii(self) -> &'static str {
        match self {
            Self::Prompt => ">",
            Self::Check => "[ok]",
            Self::Cross => "[x]",
            Self::Denied => "[no]",
            Self::Ask => "[?]",
            Self::Bullet => "*",
            Self::Paused => "[||]",
            Self::Stop => "[#]",
            Self::Edit => "[ed]",
            Self::Hint => "[h]",
            Self::TopLeft | Self::TopRight | Self::BottomLeft | Self::BottomRight => "+",
            Self::Rule => "-",
            Self::Edge => "|",
            Self::Selected => ">",
            Self::Collapsed => "v",
        }
    }
}

/// Which glyph set a renderer draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Charset {
    /// The UTF-8 set of `02` §11.
    Utf8,
    /// The ASCII fallback, for `--ascii` or a non-UTF-8 locale.
    Ascii,
}

/// Chooses the charset: `--ascii` forces the fallback, and so does a locale
/// that is not UTF-8 (`02` §11).
pub(crate) fn charset(ascii_flag: bool, utf8_locale: bool) -> Charset {
    if ascii_flag || !utf8_locale {
        Charset::Ascii
    } else {
        Charset::Utf8
    }
}

/// The glyph to draw for `glyph` in `charset`.
pub(crate) fn glyph(glyph: Glyph, charset: Charset) -> &'static str {
    match charset {
        Charset::Utf8 => glyph.utf8(),
        Charset::Ascii => glyph.ascii(),
    }
}
