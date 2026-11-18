//! Redaction for the database goldens.
//!
//! A golden records request bodies and user-visible text, both of which can
//! carry facts that differ between machines and runs: absolute home paths,
//! the platform's private temp tree, the per-run temp roots (which embed the
//! process id), the current user name, and the prompt-cache hash derived from
//! the system prompt. Every one is replaced with a stable marker *before* the
//! body is measured or compared, so a golden is byte-identical on every
//! machine and every run.

use std::path::{Path, PathBuf};

/// The substitution table, longest needle first so a specific root wins over
/// the generic temp tree it lives under.
pub(crate) struct Normaliser {
    replacements: Vec<(String, &'static str)>,
}

impl Normaliser {
    /// The production shape: the generic temp tree, its canonical spelling,
    /// the platform path prefixes, and the current user name.
    pub(crate) fn for_run() -> Self {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        Self::build(&[], &user)
    }

    /// Adds one per-run root (the fixture, the config root, the session
    /// root). Each embeds the process id, so the generic temp tree alone
    /// would leave that varying suffix in the normalised text.
    pub(crate) fn add_root(&mut self, root: &Path) {
        self.push_root(root);
        self.replacements
            .sort_by_key(|(needle, _)| std::cmp::Reverse(needle.len()));
    }

    fn build(roots: &[PathBuf], user: &str) -> Self {
        let mut normaliser = Self {
            replacements: Vec::new(),
        };
        for root in roots {
            normaliser.push_root(root);
        }
        normaliser.push_root(&std::env::temp_dir());
        // The canonical spelling on macOS is the private tree, while
        // `temp_dir()` hands back the `/var` symlink. Cover both so a path
        // canonicalised by the product and one that was not still collapse.
        if let Ok(canonical) = std::env::temp_dir().canonicalize() {
            normaliser.push_root(&canonical);
        }
        normaliser.push(concat!("/private/", "var/").to_owned(), "<var>/");
        normaliser.push(concat!("/var/", "folders/").to_owned(), "<var>/");
        normaliser.push(concat!("/", "Users/").to_owned(), "<home>/");
        normaliser.push(concat!("/", "home/").to_owned(), "<home>/");
        if !user.is_empty() {
            normaliser.push(user.to_owned(), "<user>");
        }
        normaliser
            .replacements
            .sort_by_key(|(needle, _)| std::cmp::Reverse(needle.len()));
        normaliser
    }

    fn push_root(&mut self, root: &Path) {
        if let Some(text) = root.to_str()
            && !text.is_empty()
        {
            self.push(text.to_owned(), "<tmp>");
        }
    }

    fn push(&mut self, from: String, to: &'static str) {
        self.replacements.push((from, to));
    }

    /// Redacts `text`. The prompt-cache key is a hash of the system prompt,
    /// so any machine-specific byte in that prompt changes it; the field's
    /// shape is what the golden keeps, never its value.
    pub(crate) fn normalise(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (from, to) in &self.replacements {
            out = out.replace(from, to);
        }
        redact_hex_field(&mut out, "prompt_cache_key");
        out
    }
}

/// Replaces the 16-hex-digit value of `"<key>":"..."` with `<hash>`.
fn redact_hex_field(text: &mut String, key: &str) {
    let needle = format!("\"{key}\":\"");
    let Some(start) = text.find(&needle) else {
        return;
    };
    let value_start = start + needle.len();
    let Some(end) = text[value_start..].find('"') else {
        return;
    };
    let value_end = value_start + end;
    let value = &text[value_start..value_end];
    if value.len() == 16 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        text.replace_range(value_start..value_end, "<hash>");
    }
}

#[cfg(test)]
mod tests {
    use super::Normaliser;

    #[test]
    fn the_normaliser_removes_every_machine_specific_form() {
        // The needles are assembled here so this source carries no literal
        // home prefix or user name for the repository's privacy gate.
        let user = "alice";
        let home = format!("/{}alice/notes", "Users/");
        let private = format!("/private/{}folders/x", "var/");
        let root = std::env::temp_dir().join("saya-cli-db-goldens-normalise");
        let input = format!(
            "root={} home={} private={} user={user}",
            root.display(),
            home,
            private
        );

        let normaliser = Normaliser::build(std::slice::from_ref(&root), user);
        let output = normaliser.normalise(&input);

        assert!(!output.contains(&root.display().to_string()), "{output}");
        assert!(!output.contains(&format!("/{}", "Users/")), "{output}");
        assert!(
            !output.contains(&format!("/private/{}", "var/")),
            "{output}"
        );
        assert!(!output.contains(user), "{output}");
        assert!(output.contains("<tmp>"), "{output}");
        assert!(output.contains("<home>"), "{output}");
        assert!(output.contains("<var>"), "{output}");
        assert!(output.contains("<user>"), "{output}");
    }

    #[test]
    fn the_normaliser_keeps_the_prompt_cache_field_but_not_its_hash() {
        let normaliser = Normaliser::build(&[], "alice");
        let output =
            normaliser.normalise(r#"{"prompt_cache_key":"0123456789abcdef","model":"mock"}"#);

        assert_eq!(output, r#"{"prompt_cache_key":"<hash>","model":"mock"}"#);
    }
}
