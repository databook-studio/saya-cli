// The run behind `/undo` and `/rewind`, faked over one scratch folder: a
// restore really writes bytes, so a test reads back what the operation changed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::{Checkpoint, Checkpoints, Restored};

/// The workspace-relative path every case edits.
pub(super) const EDITED: &str = "notes.txt";

/// One checkpoint the fake run holds: its number, the words the surface can
/// name for it, and the file state it recorded.
pub(super) struct Recorded {
    pub(super) turn: u32,
    pub(super) words: Option<&'static str>,
    pub(super) bytes: &'static str,
}

/// A throwaway folder removed when the case ends.
pub(super) struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A fresh folder holding the one edited file at `bytes`.
pub(super) fn case(name: &str, bytes: &str) -> (Scratch, PathBuf) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-k6-undo-{name}-{}-{stamp}", process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(EDITED), bytes).unwrap();
    (Scratch(folder.clone()), folder)
}

/// A run's checkpoints over a folder: `restore` writes the recorded bytes back,
/// which is what the one rewind operation does to the working checkout.
pub(super) struct FakeRun {
    folder: PathBuf,
    recorded: Vec<Recorded>,
    restored: Mutex<Vec<u32>>,
    /// The paths another run changed since: reported, never written.
    changed_elsewhere: Vec<String>,
}

impl FakeRun {
    pub(super) fn new(folder: PathBuf, recorded: Vec<Recorded>) -> Self {
        Self {
            folder,
            recorded,
            restored: Mutex::new(Vec::new()),
            changed_elsewhere: Vec::new(),
        }
    }

    /// The same run, with `paths` changed by another run since it last wrote
    /// them: the restore leaves them as they stand.
    pub(super) fn with_changed_elsewhere(mut self, paths: &[&str]) -> Self {
        self.changed_elsewhere = paths.iter().map(|path| (*path).to_owned()).collect();
        self
    }

    /// The edited file as it now stands.
    pub(super) fn text(&self) -> String {
        fs::read_to_string(self.folder.join(EDITED)).unwrap()
    }

    /// Every checkpoint number a restore was asked for, in order.
    pub(super) fn restored(&self) -> Vec<u32> {
        self.restored.lock().unwrap().clone()
    }
}

#[async_trait]
impl Checkpoints for FakeRun {
    async fn checkpoints(&self) -> Result<Vec<Checkpoint>, String> {
        Ok(self
            .recorded
            .iter()
            .map(|recorded| Checkpoint {
                turn: recorded.turn,
                files: 1,
                first_words: recorded.words.map(str::to_owned),
            })
            .collect())
    }

    async fn restore(&self, turn: u32) -> Result<Restored, String> {
        let recorded = self
            .recorded
            .iter()
            .find(|recorded| recorded.turn == turn)
            .ok_or_else(|| format!("no checkpoint {turn}"))?;
        self.restored.lock().unwrap().push(turn);
        let changed = self.changed_elsewhere.clone();
        let restored = if changed.iter().any(|path| path == EDITED) {
            Vec::new()
        } else {
            fs::write(self.folder.join(EDITED), recorded.bytes).unwrap();
            vec![EDITED.to_owned()]
        };
        Ok(Restored {
            restored,
            changed_elsewhere: changed,
        })
    }
}

/// Two editing turns: the run checkpointed the file after each of them, so the
/// newest holds what the last turn left and the one before it is the state that
/// turn began with.
pub(super) fn two_turns(folder: PathBuf) -> FakeRun {
    FakeRun::new(
        folder,
        vec![
            Recorded {
                turn: 1,
                words: Some("start the notes"),
                bytes: "first\n",
            },
            Recorded {
                turn: 2,
                words: Some("add the second line"),
                bytes: "second\n",
            },
        ],
    )
}

/// Three editing turns, each with words the listing can be ordered by.
pub(super) fn three_turns(folder: PathBuf) -> FakeRun {
    FakeRun::new(
        folder,
        vec![
            Recorded {
                turn: 1,
                words: Some("oldest words"),
                bytes: "first\n",
            },
            Recorded {
                turn: 2,
                words: Some("middle words"),
                bytes: "second\n",
            },
            Recorded {
                turn: 3,
                words: Some("newest words"),
                bytes: "third\n",
            },
        ],
    )
}
