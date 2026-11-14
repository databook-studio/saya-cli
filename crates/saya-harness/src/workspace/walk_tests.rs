//! A real concurrent directory replacement must not redirect a held DFS parent.
use super::*;
use std::{fs, io::Read};
#[test]
fn descent_keeps_the_actual_held_parent_after_replacement() {
    let root = std::env::temp_dir().join(format!(
        "saya-search-parent-race-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    fs::create_dir_all(root.join("parent/child")).unwrap();
    fs::write(root.join("parent/child/marker.txt"), b"held original\n").unwrap();
    let workspace = Workspace::open(&root).unwrap();
    struct Race<'a> {
        root: &'a std::path::Path,
        replaced: bool,
        observed: Option<String>,
    }
    impl WalkVisitor for Race<'_> {
        fn seen(&mut self) -> Result<bool, HarnessError> {
            Ok(true)
        }
        fn entry(
            &mut self,
            path: &str,
            kind: EntryKind,
            _size: u64,
            _mtime: Option<std::time::SystemTime>,
            directory: &Directory,
        ) -> Result<WalkAction, HarnessError> {
            if path == "moved-parent" || (path == "parent" && self.replaced) {
                return Ok(WalkAction::Prune);
            }
            if path == "parent/child" && kind == EntryKind::Dir && !self.replaced {
                fs::rename(self.root.join("parent"), self.root.join("moved-parent")).unwrap();
                fs::create_dir_all(self.root.join("parent/child")).unwrap();
                fs::write(self.root.join("parent/child/marker.txt"), b"replacement\n").unwrap();
                self.replaced = true;
            }
            if path == "parent/child/marker.txt" {
                let (mut file, _) = directory.file(path, "marker.txt").unwrap().unwrap();
                let mut bytes = String::new();
                file.read_to_string(&mut bytes).unwrap();
                self.observed.get_or_insert(bytes);
            }
            Ok(WalkAction::Continue)
        }
    }
    let mut race = Race {
        root: &root,
        replaced: false,
        observed: None,
    };
    workspace.walk_stream("", &mut race).unwrap();
    let replaced = race.replaced;
    let observed = race.observed.take();
    fs::remove_dir_all(root).unwrap();
    assert!(
        replaced,
        "actual parent replacement must occur before child descent"
    );
    assert_eq!(
        observed.as_deref(),
        Some("held original\n"),
        "descent must use its held parent, not reopen the replacement"
    );
}
