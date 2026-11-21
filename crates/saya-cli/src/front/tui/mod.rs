//! The new TUI (`06` §5, units T0-T20).
//!
//! T0 lands the theme and nothing else: semantic tokens, the glyph set and the
//! `--ascii` fallback as pure data, so the screens that follow name meaning
//! instead of hue. No rendering, no widgets, no event loop.

pub(crate) mod theme;

#[allow(unused_imports)]
pub(crate) use theme::{Charset, Glyph, Scheme, Token, bypass_chip, charset, form, glyph, style};
