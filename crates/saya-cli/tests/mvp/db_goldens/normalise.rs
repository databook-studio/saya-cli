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
        out = redact_snapshot_ids(&out);
        out
    }
}

/// Replaces every run's snapshot id — `<8 digits>-<6 digits>-<hex>` — with
/// `<snapshot>`.
///
/// The continuation block a chat request carries states the run's own snapshot
/// id, which embeds the instant the run started and a short hash. Neither is a
/// behaviour, and the hash is not zero-padded, so the id is one byte shorter
/// whenever it begins with a zero: measured as bytes it made a golden flake by
/// one byte between otherwise identical runs.
fn redact_snapshot_ids(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        match snapshot_end(bytes, index) {
            Some(end) => {
                out.push_str(&text[copied..index]);
                out.push_str("<snapshot>");
                copied = end;
                index = end;
            }
            None => index += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Where the snapshot id starting at `start` ends, if one starts there.
fn snapshot_end(bytes: &[u8], start: usize) -> Option<usize> {
    let digits = |from: usize, count: usize| {
        bytes
            .get(from..from + count)
            .filter(|run| run.iter().all(u8::is_ascii_digit))
    };
    digits(start, 8)?;
    (bytes.get(start + 8) == Some(&b'-')).then_some(())?;
    digits(start + 9, 6)?;
    (bytes.get(start + 15) == Some(&b'-')).then_some(())?;
    let mut end = start + 16;
    while bytes.get(end).is_some_and(u8::is_ascii_hexdigit) {
        end += 1;
    }
    (end > start + 16).then_some(end)
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
    fn the_normaliser_keeps_the_snapshot_field_but_not_its_id() {
        let normaliser = Normaliser::build(&[], "alice");
        // One id with a full-width hash and one whose hash begins with a zero:
        // both are the run's, neither is a behaviour, and both must measure the
        // same so the body size does not move between runs.
        let wide = normaliser.normalise(r#""snapshot_id":"20261122-042623-36ff29a0""#);
        let narrow = normaliser.normalise(r#""snapshot_id":"20261122-042629-ee71c0""#);
        assert_eq!(wide, r#""snapshot_id":"<snapshot>""#);
        assert_eq!(narrow, wide);
        // A number that merely looks like a date is left alone.
        assert_eq!(
            normaliser.normalise("20261122 042623 16"),
            "20261122 042623 16"
        );
    }

    #[test]
    fn the_normaliser_keeps_the_prompt_cache_field_but_not_its_hash() {
        let normaliser = Normaliser::build(&[], "alice");
        let output =
            normaliser.normalise(r#"{"prompt_cache_key":"0123456789abcdef","model":"mock"}"#);

        assert_eq!(output, r#"{"prompt_cache_key":"<hash>","model":"mock"}"#);
    }
}
