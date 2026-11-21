//! The row's words: what each segment says, and the token it says it in.
//!
//! Every segment is named here once, in the order the row reads them. Which of
//! them may give way — and in what order, and which never may — is
//! [`super::status_fit`]'s; this module only says what the row would like to
//! show.

use saya_agent::PermissionMode;

use super::status::StatusFacts;
use super::status_fit::{Segment, Step};
use crate::front::tui::theme::{BYPASS_CHIP, Charset, Glyph, Token, bypass_chip, glyph, style};
use crate::interactive::session::permissions::mode;

/// What the row says when no database is attached.
pub(super) const NO_DATABASE: &str = "no database";

/// What the row says when nothing is bound to the session.
pub(super) const NO_WORKSPACE: &str = "no workspace";

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

/// The left group, left to right: the chip, what it talks to, where, and every
/// condition worth a caveat.
pub(super) fn segments(facts: &StatusFacts) -> Vec<Segment> {
    let mut segments = vec![chip(facts), database(facts)];
    if !facts.model.is_empty() {
        segments.push(Segment::new(
            facts.model.clone(),
            style(Token::Dim, facts.scheme),
            Some(Step::Model),
        ));
    }
    segments.push(Segment::new(
        match &facts.workspace {
            Some(root) => root.clone(),
            None => NO_WORKSPACE.to_owned(),
        },
        style(Token::Dim, facts.scheme),
        Some(Step::Workspace),
    ));
    if facts.host_unsandboxed {
        segments.push(Segment::new(
            format!("{} host unsandboxed", caveat(facts.charset)),
            style(Token::Warn, facts.scheme),
            None,
        ));
    }
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
    if !facts.deny.is_empty() {
        segments.push(Segment::new(
            format!(
                "{} deny: {}",
                glyph(Glyph::Denied, facts.charset),
                facts.deny.join(",")
            ),
            style(Token::Dim, facts.scheme),
            Some(Step::Deny),
        ));
    }
    if let Some(percent) = facts.context_percent {
        segments.push(Segment::new(
            format!("ctx {percent}%"),
            style(Token::Dim, facts.scheme),
            Some(Step::Context),
        ));
    }
    if facts.jobs > 0 {
        segments.push(Segment::new(
            format!("jobs {}", facts.jobs),
            style(Token::Dim, facts.scheme),
            Some(Step::Jobs),
        ));
    }
    segments
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
/// absence in words. Of all the left group it is the one that gives way in
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
