//! Symlink target validation: the snapshot stores a symlink as its target
//! string, so the target must be a bounded UTF-8 relative path that resolves
//! inside the repository root. A target is resolved component by component
//! against the snapshot's whole link set, so a component that is itself a
//! snapshot symlink is substituted with its validated target. Nothing here
//! follows a link on the filesystem.

use std::collections::{BTreeMap, VecDeque};

use unicode_normalization::UnicodeNormalization;

use super::{SnapshotError, types::MAX_SYMLINK_TARGET_BYTES};

/// The most symlink substitutions one target may need. A longer chain, and
/// every cycle, is refused: the target cannot be proven to stay in the root.
pub(crate) const MAX_SYMLINK_HOPS: usize = 40;

/// The snapshot's whole symlink set: path -> raw target bytes. Validating the
/// set resolves every target through every other link, so a chain cannot hide
/// an escape behind an in-root first hop.
#[derive(Default)]
pub(crate) struct SymlinkSet {
    links: BTreeMap<String, Vec<u8>>,
    /// The folded form of every link path. A component is looked up here when
    /// its exact bytes name no link, so the resolver treats `d/UP` as `d/up`
    /// on a case-insensitive filesystem. One folded key can name more than
    /// one exact path; such a lookup is refused rather than guessed.
    folded: BTreeMap<String, Vec<String>>,
}

impl SymlinkSet {
    /// Records one link. The target is not validated here; [`Self::validate`]
    /// validates the whole set.
    pub(crate) fn insert(&mut self, path: &str, target: Vec<u8>) {
        if self.links.insert(path.to_owned(), target).is_none() {
            self.folded
                .entry(fold(path))
                .or_default()
                .push(path.to_owned());
        }
    }

    /// Validates every link's syntax and its component-by-component
    /// resolution through the whole set. Refusals name the link being
    /// validated, never a substituted link.
    pub(crate) fn validate(&self) -> Result<(), SymlinkRefusal> {
        for (path, target) in &self.links {
            resolve(path, target, &self.links, &self.folded)?;
        }
        Ok(())
    }
}

/// One case- and normalization-folded path key. Folding covers the two
/// filesystem behaviours that make an exact-byte comparison unsound: APFS and
/// NTFS compare case-insensitively, and both normalize Unicode. A single
/// normal form makes composed and decomposed spellings one key.
fn fold(path: &str) -> String {
    path.to_lowercase().as_str().nfc().collect()
}

/// What a resolved component names in the link set.
enum Lookup<'a> {
    None,
    Exact(&'a Vec<u8>),
    /// The target of the one link that folds to the component; the
    /// component's bytes are not that link's path.
    Folded(&'a Vec<u8>),
    /// More than one distinct link folds to the component.
    Ambiguous,
}

fn lookup<'a>(
    links: &'a BTreeMap<String, Vec<u8>>,
    folded: &'a BTreeMap<String, Vec<String>>,
    key: &str,
) -> Lookup<'a> {
    if let Some(target) = links.get(key) {
        return Lookup::Exact(target);
    }
    match folded.get(&fold(key)) {
        None => Lookup::None,
        Some(paths) if paths.len() == 1 => match links.get(paths[0].as_str()) {
            Some(target) => Lookup::Folded(target),
            None => Lookup::None,
        },
        Some(_) => Lookup::Ambiguous,
    }
}

/// One refused symlink: the link path and a static reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SymlinkRefusal {
    pub(crate) path: String,
    pub(crate) reason: &'static str,
}

/// Validates one symlink target for the link at `path` without a link set.
/// The target must be a non-empty UTF-8 string of at most
/// [`MAX_SYMLINK_TARGET_BYTES`], must not be absolute, and must not contain a
/// backslash or NUL. `..` may cancel a component but never rise above the
/// repository root, and no component may be `.git` (case-insensitively). The
/// resolved path itself may be the root.
pub(crate) fn validate_symlink_target(path: &str, target: &[u8]) -> Result<(), SnapshotError> {
    resolve(path, target, &BTreeMap::new(), &BTreeMap::new())
        .map_err(|refusal| SnapshotError::unsafe_symlink(&refusal.path, refusal.reason))
}

/// Resolves `target` from the parent of `path`, substituting every component
/// that names a link in `links`, exactly or after folding. The walk refuses
/// the moment it rises above the root, enters `.git`, follows a folded name
/// into `..`, or exceeds [`MAX_SYMLINK_HOPS`] substitutions.
fn resolve(
    path: &str,
    target: &[u8],
    links: &BTreeMap<String, Vec<u8>>,
    folded: &BTreeMap<String, Vec<String>>,
) -> Result<(), SymlinkRefusal> {
    let refuse = |reason| SymlinkRefusal {
        path: path.to_owned(),
        reason,
    };
    if target.is_empty() {
        return Err(refuse("the target is empty"));
    }
    if target.len() > MAX_SYMLINK_TARGET_BYTES {
        return Err(refuse("the target is longer than 4096 bytes"));
    }
    let target =
        std::str::from_utf8(target).map_err(|_| refuse("the target is not valid UTF-8"))?;
    check_target_text(target).map_err(refuse)?;
    let mut resolved: Vec<&str> = path.split('/').collect();
    resolved.pop();
    let mut key = resolved.join("/");
    let mut pending: VecDeque<&str> = target.split('/').collect();
    let mut hops = 0usize;
    while let Some(component) = pending.pop_front() {
        match component {
            "" | "." => {}
            ".." => {
                if !pop_component(&mut resolved, &mut key) {
                    return Err(refuse("the target escapes the repository"));
                }
            }
            name if name.eq_ignore_ascii_case(".git") => {
                return Err(refuse("the target enters .git"));
            }
            name => {
                push_component(&mut resolved, &mut key, name);
                let next = match lookup(links, folded, key.as_str()) {
                    Lookup::None => continue,
                    Lookup::Ambiguous => {
                        return Err(refuse(
                            "the target matches more than one link under case folding",
                        ));
                    }
                    Lookup::Exact(next) => next,
                    Lookup::Folded(next) => {
                        // Defence in depth: on a case-insensitive filesystem the
                        // folded name is the link, so a `..` after it climbs
                        // through the substituted target. Refuse instead of
                        // relying on the substitution's own bounds.
                        if pending.front() == Some(&"..") {
                            return Err(refuse("the target follows a case variant into `..`"));
                        }
                        next
                    }
                };
                hops += 1;
                if hops > MAX_SYMLINK_HOPS {
                    return Err(refuse(
                        "the target forms a symlink loop or chain that is too long",
                    ));
                }
                pop_component(&mut resolved, &mut key);
                let next = std::str::from_utf8(next)
                    .map_err(|_| refuse("the target is not valid UTF-8"))?;
                check_target_text(next).map_err(refuse)?;
                for part in next.split('/').rev() {
                    pending.push_front(part);
                }
            }
        }
    }
    Ok(())
}

/// Pushes one resolved component and keeps the joined key in step.
fn push_component<'a>(resolved: &mut Vec<&'a str>, key: &mut String, name: &'a str) {
    resolved.push(name);
    if !key.is_empty() {
        key.push('/');
    }
    key.push_str(name);
}

/// Pops the last resolved component (and its key segment). Returns false at
/// the repository root.
fn pop_component(resolved: &mut Vec<&str>, key: &mut String) -> bool {
    let Some(name) = resolved.pop() else {
        return false;
    };
    match key.len().checked_sub(name.len()) {
        Some(0) => key.clear(),
        Some(position) if key.as_bytes().get(position - 1) == Some(&b'/') => {
            key.truncate(position - 1);
        }
        _ => key.clear(),
    }
    true
}

/// The textual rules shared by an original target and a substituted one.
fn check_target_text(target: &str) -> Result<(), &'static str> {
    if target.is_empty() {
        return Err("the target is empty");
    }
    if target.len() > MAX_SYMLINK_TARGET_BYTES {
        return Err("the target is longer than 4096 bytes");
    }
    if target.starts_with('/') {
        return Err("the target is absolute");
    }
    if target.contains('\\') || target.as_bytes().contains(&0) {
        return Err("the target contains a backslash or NUL");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_SYMLINK_HOPS, SymlinkSet, validate_symlink_target};

    #[test]
    fn relative_targets_inside_the_root_are_accepted() {
        for target in [
            "tracked.txt",
            "dir/file",
            "../sibling",
            "./dir/../file",
            ".",
        ] {
            assert!(
                validate_symlink_target("dir/link", target.as_bytes()).is_ok(),
                "{target}"
            );
        }
    }

    #[test]
    fn absolute_escaping_and_git_targets_are_refused() {
        for target in ["/etc/passwd", "../../outside", ".git/config", ".GIT/config"] {
            assert!(
                validate_symlink_target("dir/link", target.as_bytes()).is_err(),
                "{target}"
            );
        }
    }

    #[test]
    fn non_utf8_and_oversize_targets_are_refused() {
        assert!(validate_symlink_target("link", &[0xff]).is_err());
        assert!(validate_symlink_target("link", &[b'a'; 4097]).is_err());
        assert!(validate_symlink_target("link", &[b'a'; 4096]).is_ok());
    }

    fn set(links: &[(&str, &str)]) -> SymlinkSet {
        let mut set = SymlinkSet::default();
        for (path, target) in links {
            set.insert(path, target.as_bytes().to_vec());
        }
        set
    }

    #[test]
    fn chain_that_hides_an_escape_is_refused() {
        let links = set(&[("d/up", ".."), ("d/esc", "up/../..")]);
        let refusal = links.validate().unwrap_err();
        assert_eq!(refusal.path, "d/esc");
        assert_eq!(refusal.reason, "the target escapes the repository");
    }

    #[test]
    fn in_root_two_link_chain_is_accepted() {
        assert!(
            set(&[("first", "tracked.txt"), ("second", "first")])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn symlink_loop_is_refused() {
        let refusal = set(&[("a", "b"), ("b", "a")]).validate().unwrap_err();
        assert_eq!(
            refusal.reason,
            "the target forms a symlink loop or chain that is too long"
        );
    }

    #[test]
    fn chain_over_the_hop_bound_is_refused() {
        let mut links: Vec<(String, String)> = (0..=MAX_SYMLINK_HOPS + 1)
            .map(|index| (format!("l{index}"), format!("l{}", index + 1)))
            .collect();
        links.push((format!("l{}", MAX_SYMLINK_HOPS + 2), "file".to_owned()));
        let mut set = SymlinkSet::default();
        for (path, target) in links {
            set.insert(&path, target.into_bytes());
        }
        assert_eq!(
            set.validate().unwrap_err().reason,
            "the target forms a symlink loop or chain that is too long"
        );
    }
}
