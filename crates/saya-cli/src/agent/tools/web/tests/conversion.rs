//! The conversion: an HTML document becomes readable text, and anything else
//! is passed through exactly as the server sent it.

use super::super::text;

/// A document becomes its text: tags dropped, entities decoded, and the block
/// structure the markup asked for kept as lines.
#[test]
fn an_html_document_becomes_its_text() {
    let html = "<!doctype html><html><head><title>Docs</title>\
        <style>body { color: red }</style></head>\
        <body><h1>Heading</h1><p>First &amp; second.</p>\
        <ul><li>one</li><li>two</li></ul></body></html>";
    let text = text::render(html, Some("text/html; charset=utf-8"));
    assert!(text.contains("Heading"), "{text}");
    assert!(text.contains("First & second."), "entities decode: {text}");
    assert!(
        text.contains("- one"),
        "a list item keeps its marker: {text}"
    );
    assert!(text.contains("- two"), "{text}");
    assert!(
        !text.contains("color: red") && !text.contains("body {"),
        "a stylesheet is not text: {text}"
    );
    assert!(!text.contains('<'), "no markup survives: {text}");
    assert!(
        text.find("Heading").unwrap() < text.find("First").unwrap(),
        "the document's order is kept: {text}"
    );
}

/// The elements whose content is not text are skipped whole, however they are
/// spelled, and a script's body never reaches the model.
#[test]
fn script_and_style_bodies_are_skipped_whole() {
    let html = "<html><body>before<script type=\"text/javascript\">\
        var x = \"<p>not text</p>\";</script><p>after</p>\
        <NOSCRIPT>hidden</NOSCRIPT></body></html>";
    let text = text::render(html, Some("text/html"));
    assert!(text.contains("before"), "{text}");
    assert!(text.contains("after"), "{text}");
    assert!(
        !text.contains("var x"),
        "a script's body is not text: {text}"
    );
    assert!(!text.contains("not text"), "{text}");
    assert!(
        !text.contains("hidden"),
        "an uppercase element is skipped too: {text}"
    );
}

/// A `<pre>` block keeps the page's own whitespace, which is the one place a
/// browser does too.
#[test]
fn a_pre_block_keeps_its_whitespace() {
    let html = "<p>prose</p><pre>line one\n    indented\nline three</pre><p>more</p>";
    let text = text::render(html, Some("text/html"));
    assert!(
        text.contains("line one\n    indented\nline three"),
        "{text}"
    );
}

/// Whitespace outside `<pre>` collapses the way a browser collapses it: a run
/// of spaces and newlines is one space between words.
#[test]
fn whitespace_outside_pre_collapses() {
    let html = "<p>one\n\n     two\t\tthree</p>";
    assert_eq!(text::render(html, Some("text/html")), "one two three");
}

/// An entity the table does not know — and one a bounded read cut in half — is
/// left exactly as the page wrote it. The converter never guesses.
#[test]
fn an_unknown_entity_is_left_alone() {
    let html = "<p>&amp; &nosuchthing; &half";
    let text = text::render(html, Some("text/html"));
    assert!(text.contains("& "), "a known entity decodes: {text}");
    assert!(text.contains("&nosuchthing;"), "{text}");
    assert!(
        text.contains("&half"),
        "an unterminated entity survives: {text}"
    );
}

/// A numeric character reference decodes in both spellings.
#[test]
fn numeric_entities_decode() {
    let text = text::render("<p>&#8212;&#x2014;&#65;</p>", Some("text/html"));
    assert_eq!(text, "——A");
}

/// A bounded read can cut a document mid-tag. The remainder is markup, not
/// text, and is dropped rather than guessed at.
#[test]
fn a_document_cut_mid_tag_loses_only_the_cut_tag() {
    let text = text::render("<p>kept</p><a href=\"https://exa", Some("text/html"));
    assert_eq!(text, "kept");
}

/// Anything that is not an HTML document is passed through byte for byte —
/// the server's own type is what says what the bytes mean.
#[test]
fn a_non_html_body_is_passed_through() {
    for content_type in [
        "text/plain",
        "application/json",
        "text/markdown",
        "application/xml",
    ] {
        let body = "<not html> & <markup>";
        assert_eq!(
            text::render(body, Some(content_type)),
            body,
            "{content_type} is not converted"
        );
    }
}

/// Where the server sent no type at all, the document's own first bytes decide
/// — and only then.
#[test]
fn an_untyped_body_is_sniffed() {
    assert_eq!(
        text::render("  <!DOCTYPE html><p>hi</p>", None),
        "hi",
        "a document that declares itself is rendered"
    );
    assert_eq!(text::render("<html><body>hi</body></html>", None), "hi");
    assert_eq!(
        text::render("a < b and c > d", None),
        "a < b and c > d",
        "a body that is not a document is passed through"
    );
}
