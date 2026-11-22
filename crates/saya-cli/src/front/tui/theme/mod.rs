//! The new TUI's theme: semantic tokens, the glyph set and the ASCII fallback.
//!
//! Pure data and lookups (`06` §5, T0). Nothing here renders, and no widget
//! reads it yet: the screens that follow name a [`Token`] or a [`Glyph`] and
//! get the right form for the light palette, the dark palette, and colour off.

mod ascii;
mod contrast;
mod glyphs;
mod launch;
mod tokens;

#[allow(unused_imports)]
pub(crate) use ascii::line as ascii_line;
#[allow(unused_imports)]
pub(crate) use contrast::{
    FLOOR_DIM, FLOOR_ON_SURFACE, FLOOR_TEXT, GROUND_DARK, GROUND_LIGHT, contrast_ratio,
    relative_luminance,
};
#[allow(unused_imports)]
pub(crate) use glyphs::{Charset, Glyph, charset, glyph};
#[allow(unused_imports)]
pub(crate) use launch::{NO_COLOR, asks_for_no_colour, scheme, scheme_for};
#[allow(unused_imports)]
pub(crate) use tokens::{BYPASS_CHIP, Form, Scheme, Token, bypass_chip, form, style};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
