use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use saya_harness::coding::skills::{SkillOrigin, SkillRoot};
use saya_harness::workspace::Workspace;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

pub(super) struct TempTree(PathBuf);

impl TempTree {
    pub(super) fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("saya-c05-{}-{id}", std::process::id()));
        match fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("stale test tree should be removable: {error}"),
        }
        fs::create_dir_all(&path).expect("temporary test tree should be created");
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }

    pub(super) fn workspace(&self) -> Workspace {
        Workspace::open(self.path()).expect("temporary root should open as workspace")
    }

    pub(super) fn skill(&self, directory: &str, name: &str, description: &str, body: &[u8]) {
        let path = self.path().join(directory);
        fs::create_dir_all(&path).expect("skill directory should be created");
        let mut source =
            format!("---\nname: {name}\ndescription: {description}\n---\n").into_bytes();
        source.extend_from_slice(body);
        fs::write(path.join("SKILL.md"), source).expect("skill source should be written");
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn root(tree: &TempTree, origin: SkillOrigin, label: &str) -> SkillRoot {
    SkillRoot::new(tree.workspace(), origin, label).expect("valid root label should be accepted")
}
