//! The body a `web_fetch` result carries: an HTML document rendered as
//! readable text, anything else passed through byte for byte.
//!
//! There is no HTML parser here and none is wanted. A fetched page is
//! untrusted bytes whose *markup* is not the answer, so the converter drops
//! the tags, keeps the text, and states its own limits:
//!
//! - **One pass, one buffer.** The walker never backtracks and never recurses,
//!   and its input is already bounded by the read the tool performed, so a
//!   hostile page cannot make it work harder than its own length.
//! - **Forgiving at the edges.** A bounded read can cut a document mid-tag, and
//!   real pages are not well-formed. An unterminated tag at the end is dropped
//!   rather than guessed at, and an unknown element contributes its text and no
//!   structure.
//! - **No guessing at meaning.** An entity the table in [`super::entities`]
//!   does not know is left exactly as the page wrote it.

use super::entities::decode;

/// The media types that mean "this body is a document, render it as text".
const HTML_TYPES: &[&str] = &["text/html", "application/xhtml+xml"];

/// The elements whose content is not text. The converter skips to the matching
/// close tag, so a page's scripts and stylesheets never reach the model.
const SKIPPED: &[&str] = &["script", "style", "noscript", "template", "svg"];

/// The elements that end a line, and what goes before the next word: a
/// paragraph or a block break, a blank line before a heading, a list marker, a
/// break inside a paragraph. An element not named here contributes no
/// structure — its text simply runs on, which is what the browser does.
fn separator(name: &str) -> &'static str {
    match name {
        "br" => "\n",
        "li" => "\n- ",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "\n\n",
        "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav" | "aside"
        | "table" | "tr" | "ul" | "ol" | "dl" | "blockquote" | "pre" | "figure" | "form" | "hr"
        | "title" | "caption" => "\n",
        _ => "",
    }
}

/// Renders one response body as the text the model reads.
pub(crate) fn render(body: &str, content_type: Option<&str>) -> String {
    if is_html(content_type, body) {
        html_to_text(body)
    } else {
        body.to_owned()
    }
}

/// Whether the body is an HTML document: the server's own type says so, or —
/// where it sent none — the document's first bytes do.
fn is_html(content_type: Option<&str>, body: &str) -> bool {
    match content_type {
        Some(value) => {
            let media = value.split(';').next().unwrap_or_default().trim();
            HTML_TYPES
                .iter()
                .any(|known| media.eq_ignore_ascii_case(known))
        }
        None => {
            let head: String = body.chars().take(256).collect();
            let head = head.trim_start().to_ascii_lowercase();
            head.starts_with("<!doctype html") || head.starts_with("<html")
        }
    }
}

/// Walks the document once, emitting its text.
fn html_to_text(html: &str) -> String {
    let mut out = Renderer::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.text(&rest[..start]);
        let tail = &rest[start..];
        // A comment can hold a `>` of its own, so it is closed by its own
        // terminator rather than by the first one.
        if let Some(after) = tail.strip_prefix("<!--") {
            match after.find("-->") {
                Some(at) => {
                    rest = &after[at + 3..];
                    continue;
                }
                None => return out.finish(),
            }
        }
        let Some(end) = tail.find('>') else {
            // The read cut the document mid-tag: the remainder is markup, not
            // text, and is dropped rather than guessed at.
            return out.finish();
        };
        let tag = &tail[1..end];
        rest = &tail[end + 1..];
        let (name, closing) = tag_name(tag);
        if out.verbatim && name == "pre" && closing {
            out.end_verbatim();
        } else if SKIPPED.contains(&name.as_str()) && !closing {
            rest = skip_element(rest, &name);
        } else if !closing {
            if name == "pre" {
                out.begin_verbatim();
            }
            out.break_line(separator(&name));
        }
    }
    out.text(rest);
    out.finish()
}

/// One tag's element name and whether it is a close tag. The name is
/// lowercased, because HTML element names are case-insensitive and a page may
/// spell them any way it likes.
fn tag_name(tag: &str) -> (String, bool) {
    let tag = tag.trim_start();
    let (closing, tag) = match tag.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, tag),
    };
    let end = tag
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(tag.len());
    (tag[..end].to_ascii_lowercase(), closing)
}

/// Everything after the matching close tag of `name`, or the empty tail when
/// the document holds none — an unclosed script is the rest of the document,
/// and none of it is text.
fn skip_element<'a>(rest: &'a str, name: &str) -> &'a str {
    // `to_ascii_lowercase` maps only ASCII bytes, so every multi-byte sequence
    // is untouched and the offsets found here index `rest` correctly.
    let lowered = rest.to_ascii_lowercase();
    let Some(at) = lowered.find(&format!("</{name}")) else {
        return "";
    };
    match rest[at..].find('>') {
        Some(end) => &rest[at + end + 1..],
        None => "",
    }
}

/// The rendered text under construction, with the whitespace rules of a
/// browser rather than of the markup: runs of spaces collapse to one, and a
/// break the markup asked for is placed once however many elements asked for
/// it.
struct Renderer {
    out: String,
    /// Newlines the markup asked for that the next word has not used yet,
    /// capped at two so a run of empty elements cannot open a gap.
    newlines: usize,
    /// A space is pending: whitespace was seen, and the next word needs one
    /// before it.
    space: bool,
    /// The marker a list item asked for (`- `), placed before the next word.
    marker: Option<&'static str>,
    /// Inside `<pre>`: text is kept exactly as the page wrote it.
    verbatim: bool,
}

impl Renderer {
    fn new() -> Self {
        Self {
            out: String::new(),
            newlines: 0,
            space: false,
            marker: None,
            verbatim: false,
        }
    }

    /// One run of document text: entities decoded and, outside `<pre>`,
    /// whitespace collapsed to single spaces between words.
    fn text(&mut self, raw: &str) {
        let decoded = decode(raw);
        if self.verbatim {
            self.flush();
            self.out.push_str(&decoded);
            return;
        }
        let mut words = decoded.split_whitespace().peekable();
        if words.peek().is_some() && decoded.starts_with(char::is_whitespace) {
            self.space = true;
        }
        while let Some(word) = words.next() {
            self.word(word);
            if words.peek().is_some() {
                self.space = true;
            }
        }
        if decoded.ends_with(char::is_whitespace) {
            self.space = true;
        }
    }

    fn word(&mut self, word: &str) {
        self.flush();
        if let Some(marker) = self.marker.take() {
            if !self.out.is_empty() && !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push_str(marker);
            self.out.push_str(word);
            self.space = false;
            return;
        }
        if self.space && !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push(' ');
        }
        self.out.push_str(word);
        self.space = false;
    }

    /// A break the markup asked for: the newlines of `separator`, and the
    /// marker it carries, placed before the next word.
    fn break_line(&mut self, separator: &'static str) {
        if separator.is_empty() {
            return;
        }
        let newlines = separator.chars().take_while(|c| *c == '\n').count().min(2);
        if newlines > 0 {
            self.space = false;
            self.newlines = self.newlines.max(newlines);
        }
        let marker = &separator[newlines..];
        if !marker.is_empty() {
            self.space = false;
            self.marker = Some(marker);
        }
    }

    fn begin_verbatim(&mut self) {
        self.break_line("\n");
        self.verbatim = true;
    }

    fn end_verbatim(&mut self) {
        self.verbatim = false;
        self.break_line("\n\n");
    }

    /// Places the breaks the markup asked for. A break at the very start of the
    /// text is dropped: leading blank lines are the markup's business, not the
    /// model's.
    fn flush(&mut self) {
        let newlines = std::mem::take(&mut self.newlines);
        if newlines == 0 || self.out.is_empty() {
            return;
        }
        for _ in 0..newlines {
            self.out.push('\n');
        }
        self.space = false;
    }

    fn finish(mut self) -> String {
        let trimmed = self.out.trim_end().len();
        self.out.truncate(trimmed);
        self.out
    }
}
