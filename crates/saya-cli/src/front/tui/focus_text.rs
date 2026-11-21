//! What `c` copies from the entry the cursor holds (`02` §6), and what the
//! surface says it copied.
//!
//! The text is the entry's *source*, not its painted rows: an answer's markdown
//! as the model wrote it, a query's SQL, a command's line, a diff's rows. A
//! copy that handed the operator back the cells it had already read would be
//! worth nothing.
//!
//! The toast is `02` §4's one sentence — `copied answer (1.2 kB)` — with the
//! entry's own word and the size of what was copied.

use crate::front::view::{Clarification, DiffKind, Entry};

/// The word the toast names `entry` by, and the plain text it copies. The
/// activity group has no text of its own to copy: its words are its steps',
/// and one query's SQL is copied from the result block that holds it.
pub(crate) fn plain(entry: &Entry) -> Option<(&'static str, String)> {
    match entry {
        Entry::UserMessage(message) => Some(("message", message.text.clone())),
        Entry::Answer(answer) => Some(("answer", answer.text.clone())),
        Entry::ResultPreview(preview) => Some(("SQL", preview.sql.clone())),
        Entry::Notice(notice) => Some(("note", notice.message.clone())),
        Entry::Edit(edit) => Some(("diff", diff_text(&edit.lines))),
        Entry::Command(command) => Some(("command", command.command.clone())),
        Entry::Helper(helper) => Some(("helper", format!("{} · {}", helper.id, helper.question))),
        Entry::Error(error) => Some(("error", error.message.clone())),
        Entry::Question(question) => Some(("question", question_text(question))),
        // A stop is a composed block — a heading, the guard's sentence and the
        // limit — with no source text of its own to hand over.
        Entry::Stop(_) | Entry::Activity(_) => None,
    }
}

/// A question as it is typed: the model's words, then the answers it offered,
/// numbered as `02` §3.9 draws them.
fn question_text(question: &Clarification) -> String {
    let mut text = question.question.clone();
    for (index, option) in question.options.iter().enumerate() {
        text.push_str(&format!("\n{}. {option}", index + 1));
    }
    text
}

/// `02` §4's sentence: what was copied, and how much of it.
pub(crate) fn toast(word: &str, text: &str) -> String {
    format!("copied {word} ({})", size_text(text.len()))
}

/// A byte count in the units the toast uses: `742 B`, `1.2 kB`.
pub(crate) fn size_text(bytes: usize) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    format!("{:.1} kB", bytes as f64 / 1000.0)
}

/// A diff as a patch reads: the sign, then the line as the file holds it.
pub(crate) fn diff_text(lines: &[crate::front::view::DiffLine]) -> String {
    lines
        .iter()
        .map(|line| {
            let sign = match line.kind {
                DiffKind::Added => '+',
                DiffKind::Removed => '-',
                DiffKind::Context => ' ',
            };
            format!("{sign}{}", line.text)
        })
        .collect::<Vec<String>>()
        .join("\n")
}
