//! The row's words: what each segment says, and the token it says it in.
//!
//! Every segment is named here once, in the order the row reads them: the mode
//! chip, the model, the database, and the one caveat that changes what every
//! answer contains. What gives way — and which segments never do — is
//! [`super::status_fit`]'s; this module only says what the row would like to
//! show.

use saya_agent::PermissionMode;

use super::status::StatusFacts;
use super::status_fit::{Segment, Step};
use crate::front::tui::theme::{BYPASS_CHIP, Charset, Token, bypass_chip, style};
use crate::interactive::session::permissions::mode;

/// What the row says when no database is attached.
pub(super) const NO_DATABASE: &str = "no database";

/// The caveat mark. The glyph table names the transcript's glyphs; this one is
/// the row's own copy, so it carries its ASCII fallback beside it.
const CAVEAT: &str = "⚠";
const CAVEAT_ASCII: &str = "!";

/// The caveat mark for the active charset.
const fn caveat(charset: Charset) -> &'static str {
    match charset {
        Charset::Utf8 => CAVEAT,
        Charset::Ascii => CAVEAT_ASCII,
    }
}

/// The left group, left to right: the chip, the model, the database, and the
/// one caveat that survives a quiet default.
pub(super) fn segments(facts: &StatusFacts) -> Vec<Segment> {
    let mut segments = vec![chip(facts)];
    if let Some(model) = short_model(&facts.model) {
        segments.push(Segment::new(model, style(Token::Dim, facts.scheme), None));
    }
    segments.push(database(facts));
    if facts.sharing {
        let mark = caveat(facts.charset);
        segments.push(
            Segment::new(
                format!("{mark} data sharing on"),
                style(Token::Warn, facts.scheme),
                Some(Step::Sharing),
            )
            .shortening(format!("{mark} sharing")),
        );
    }
    segments
}

/// The model's short name: the part after the provider's `/`, because the
/// provider is the same for every turn and the model is the name the operator
/// chose. A model with no prefix reads as it is.
pub(super) fn short_model(model: &str) -> Option<String> {
    let name = model.rsplit('/').next().unwrap_or(model).trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The mode chip: the mode's own spelling, and `! BYPASS` in inverse `bad` for
/// the one mode whose word has to survive colour going away.
fn chip(facts: &StatusFacts) -> Segment {
    match facts.mode {
        PermissionMode::Bypass => {
            Segment::new(BYPASS_CHIP, bypass_chip(facts.scheme).style(), None)
        }
        other => Segment::new(
            mode::name(other),
            style(chip_token(other), facts.scheme),
            None,
        ),
    }
}

/// The chip's token per the mode: neutral at `ask`, accent for the mode that
/// edits, info for the plan, dim for the two postures that never ask, and `bad`
/// for bypass.
const fn chip_token(mode: PermissionMode) -> Token {
    match mode {
        PermissionMode::Ask => Token::Fg,
        PermissionMode::AcceptEdits => Token::Accent,
        PermissionMode::Plan => Token::Info,
        PermissionMode::ReadOnly | PermissionMode::Never => Token::Dim,
        PermissionMode::Bypass => Token::Bad,
    }
}

/// The database segment: the primary by name, the rest as a count, or the
/// absence in words. Of the row's three facts it is the one that gives way in
/// words — `demo +1` reading `+1` — because the row must always say what it is
/// talking to.
fn database(facts: &StatusFacts) -> Segment {
    let (text, short) = match facts.databases.split_first() {
        None => (NO_DATABASE.to_owned(), None),
        Some((first, [])) => (first.clone(), None),
        Some((first, rest)) => (
            format!("{first} +{}", rest.len()),
            Some(format!("+{}", rest.len())),
        ),
    };
    let mut segment = Segment::new(
        text,
        style(Token::Accent, facts.scheme),
        Some(Step::Database),
    );
    segment.short = short;
    segment
}
