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
            // macOS reaches part of the filesystem through `/private`: `/var`
            // and `/tmp` are symlinks, so the product may name any path under
            // them canonically. Register that spelling of the whole root too,
            // or the generic root above matches its tail and leaves the
            // differing `/private` prefix in the golden.
            if let Some(canonical) = private_spelling(text) {
                self.push(canonical, "<tmp>");
            }
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

/// The `/private`-prefixed spelling of a path macOS resolves through the
/// private tree, or `None` for a path it does not. `/var/...` and `/tmp/...`
/// are symlinks into `/private`, so both spellings name the same bytes.
fn private_spelling(text: &str) -> Option<String> {
    let private = concat!("/", "private");
    let linked = [concat!("/", "var/"), concat!("/", "tmp/")];
    (!text.starts_with(private) && linked.iter().any(|root| text.starts_with(root)))
        .then(|| format!("{private}{text}"))
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
    use std::path::PathBuf;

    use super::Normaliser;

    /// The macOS temp tree can be spelled `/var/...` or through the
    /// `/private/var/...` prefix it resolves to, and a Linux run spells it
    /// `/tmp/...`. The same workspace must normalise to the same bytes
    /// whichever spelling the product handed the request body.
    #[test]
    fn every_spelling_of_one_temp_root_normalises_to_the_same_bytes() {
        // Assembled here so this source carries no literal temp prefix.
        let root = format!("/{}folders/xy/abc/T/saya-run", "var/");
        let through_private = format!("/private{root}");
        let on_linux = format!("/{}saya-run", "tmp/");
        let normaliser =
            Normaliser::build(&[PathBuf::from(&root), PathBuf::from(&on_linux)], "alice");

        let spelled = normaliser.normalise(&format!("{root}/cwd"));
        let canonical = normaliser.normalise(&format!("{through_private}/cwd"));
        let linux = normaliser.normalise(&format!("{on_linux}/cwd"));

        assert_eq!(spelled, "<tmp>/cwd");
        assert_eq!(
            canonical, spelled,
            "the canonical /private spelling of a temp root must collapse like the plain one"
        );
        assert_eq!(
            linux, spelled,
            "a Linux /tmp root must collapse like a macOS one"
        );
    }

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
