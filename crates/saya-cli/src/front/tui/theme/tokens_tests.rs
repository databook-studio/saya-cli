//! `02` §12's three columns, asserted.
//!
//! The contrast rule these tests hold the forms to is stated once, in
//! `contrast.rs`: WCAG 2.1 relative luminance, a floor of 4.5:1 for body text
//! and 3:1 for `dim`, measured against the terminal's default ground — black for
//! the dark palette, white for the light one — with `Reset` forms exempt because
//! they paint no hue of their own.

use crate::front::tui::theme::{
    BYPASS_CHIP, FLOOR_DIM, FLOOR_ON_SURFACE, FLOOR_TEXT, Form, GROUND_DARK, GROUND_LIGHT, Scheme,
    Token, bypass_chip, contrast_ratio, form, style,
};
use crate::interactive::tui::ui::theme::hue;
use ratatui::style::{Color, Modifier};

#[test]
fn the_token_set_is_the_one_02_12_names() {
    let names: Vec<&str> = Token::ALL.iter().map(|token| token.name()).collect();
    assert_eq!(
        names,
        [
            "fg", "dim", "user", "accent", "info", "ok", "bad", "warn", "code", "surface", "focus",
        ],
        "the token set drifted from the table in 02 §12"
    );
}

#[test]
fn every_token_has_a_no_colour_form_that_differs_from_plain_text() {
    for token in Token::ALL {
        let resolved = form(token, Scheme::NoColour);
        assert!(
            resolved.fg.is_none() && resolved.bg.is_none(),
            "{} paints colour with colour off",
            token.name()
        );
        if token == Token::Fg {
            // The one exemption, asserted rather than assumed: with colour gone,
            // prose *is* plain text. Its meaning needs no carrier.
            assert_eq!(
                resolved,
                Form::plain(),
                "fg's no-colour form is exactly plain prose"
            );
            continue;
        }
        assert_ne!(
            resolved,
            Form::plain(),
            "{} collapses to plain text with colour off, so its meaning is lost",
            token.name()
        );
    }
}

#[test]
fn no_colour_markers_are_short_seven_bit_words() {
    for token in Token::ALL {
        let Some(marker) = form(token, Scheme::NoColour).marker else {
            continue;
        };
        assert!(
            !marker.trim().is_empty(),
            "{}'s marker is blank",
            token.name()
        );
        assert!(
            marker.is_ascii(),
            "{}'s marker {marker:?} is not seven-bit, so it is not safe with colour off",
            token.name()
        );
        assert!(
            marker.len() <= 8,
            "{}'s marker {marker:?} is too long to sit in a cell run",
            token.name()
        );
    }
}

#[test]
fn light_and_dark_forms_meet_the_contrast_floor() {
    for token in Token::ALL {
        for (scheme, ground) in [(Scheme::Dark, GROUND_DARK), (Scheme::Light, GROUND_LIGHT)] {
            if token == Token::Surface {
                // A ground, not text: the next test measures what reads on it.
                continue;
            }
            let resolved = form(token, scheme);
            let Some(fg) = resolved.fg else {
                panic!("{} paints no colour in {scheme:?}", token.name());
            };
            if fg == Color::Reset {
                // The terminal's own text on its own ground: no hue of ours to
                // measure, which is exactly why the dark palette picks it.
                continue;
            }
            let floor = if token == Token::Dim {
                FLOOR_DIM
            } else {
                FLOOR_TEXT
            };
            let ratio = contrast_ratio(fg, ground).expect("every form is an RGB hue");
            assert!(
                ratio >= floor,
                "{} in {scheme:?} reaches {ratio:.2}:1 against the ground, below the {floor}:1 floor",
                token.name()
            );
        }
    }
}

#[test]
fn every_form_reads_against_the_surface_it_paints_on() {
    for scheme in [Scheme::Dark, Scheme::Light] {
        let surface = form(Token::Surface, scheme);
        assert!(
            surface.fg.is_none(),
            "surface is a background, never a foreground ({scheme:?})"
        );
        let Some(ground) = surface.bg else {
            panic!("surface paints no ground in {scheme:?}");
        };
        for token in Token::ALL {
            if token == Token::Surface {
                continue;
            }
            let Some(fg) = form(token, scheme).fg else {
                panic!("{} paints no colour in {scheme:?}", token.name());
            };
            if fg == Color::Reset {
                continue;
            }
            let ratio = contrast_ratio(fg, ground).expect("every form is an RGB hue");
            assert!(
                ratio >= FLOOR_ON_SURFACE,
                "{} in {scheme:?} reaches {ratio:.2}:1 on surface, below {FLOOR_ON_SURFACE}:1",
                token.name()
            );
        }
    }
}

#[test]
fn the_bypass_chip_reads_bang_bypass_with_colour_off() {
    let chip = bypass_chip(Scheme::NoColour);
    assert_eq!(
        chip.marker,
        Some("! BYPASS"),
        "the bypass chip's no-colour form must read `! BYPASS`"
    );
    assert_eq!(chip.marker, Some(BYPASS_CHIP));
    assert!(
        chip.fg.is_none() && chip.bg.is_none(),
        "the chip's colour-off form paints no colour"
    );
    assert!(
        chip.modifiers.contains(Modifier::REVERSED),
        "reverse video is the chip's colour-off carrier, got {:?}",
        chip.modifiers
    );
    for scheme in [Scheme::Dark, Scheme::Light, Scheme::NoColour] {
        assert_eq!(
            bypass_chip(scheme).marker,
            Some(BYPASS_CHIP),
            "the word never drops for width or colour ({scheme:?})"
        );
        assert!(
            bypass_chip(scheme).modifiers.contains(Modifier::REVERSED),
            "the chip is inverted in every scheme ({scheme:?})"
        );
    }
}

#[test]
fn a_style_is_the_form_it_resolves_to() {
    let accent = style(Token::Accent, Scheme::Light);
    assert_eq!(accent.fg, Some(hue::ACCENT_LIGHT));
    assert!(
        accent.add_modifier.contains(Modifier::BOLD),
        "accent stays bold, so it survives colour going away"
    );
    assert_eq!(
        style(Token::Focus, Scheme::NoColour).add_modifier,
        Modifier::UNDERLINED,
        "the cursor row is underlined with colour off"
    );
    assert_eq!(
        style(Token::User, Scheme::NoColour).add_modifier,
        Modifier::BOLD,
        "the user's turn stays bold with colour off"
    );
    assert_eq!(
        style(Token::Dim, Scheme::NoColour).add_modifier,
        Modifier::DIM,
        "captions stay dimmed with colour off"
    );
}
