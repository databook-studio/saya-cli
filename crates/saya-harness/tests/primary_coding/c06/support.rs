use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use saya_harness::coding::skills::{
    SkillBody, SkillEntry, SkillLoadError, SkillOrigin, SkillRoot, discover_skills, load_skill_body,
};
use saya_harness::workspace::Workspace;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

pub(super) struct TempTree(PathBuf);

impl TempTree {
    pub(super) fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("saya-c06-{}-{id}", std::process::id()));
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
        self.write_skill_file(directory, &self.skill_bytes(name, description, body));
    }

    pub(super) fn skill_bytes(&self, name: &str, description: &str, body: &[u8]) -> Vec<u8> {
        let mut source =
            format!("---\nname: {name}\ndescription: {description}\n---\n").into_bytes();
        source.extend_from_slice(body);
        source
    }

    pub(super) fn write_skill_file(&self, directory: &str, bytes: &[u8]) {
        let path = self.path().join(directory);
        fs::create_dir_all(&path).expect("skill directory should be created");
        fs::write(path.join("SKILL.md"), bytes).expect("skill source should be written");
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

pub(super) fn entry_for(root: &SkillRoot, name: &str) -> SkillEntry {
    let index = discover_skills(std::slice::from_ref(root)).expect("discovery should succeed");
    index
        .skills
        .into_iter()
        .find(|entry| entry.name == name)
        .expect("skill should be indexed")
}

pub(super) fn load(root: &SkillRoot, entry: &SkillEntry) -> Result<SkillBody, SkillLoadError> {
    load_skill_body(std::slice::from_ref(root), entry)
}
