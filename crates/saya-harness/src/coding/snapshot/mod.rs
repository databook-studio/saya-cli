mod capture;
mod capture_blob;
mod capture_reader;
mod facts;
mod manifest;
mod materialize_target;
mod policy;
mod restore;
mod select;
mod select_support;
mod types;

pub use capture_blob::capture_head_blob;
pub use capture_reader::{CapturedFile, read_working_file};
pub use facts::{SnapshotChange, SnapshotStatusFact};
pub use manifest::{RedactedIoError, SnapshotError, SnapshotManifest};
pub use policy::SnapshotOmissionPolicy;
pub use types::{
    MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_MANIFEST_BYTES,
    MAX_SNAPSHOT_TOTAL_BYTES, SnapshotContentSource, SnapshotFileEntry, SnapshotFileMode,
    SnapshotIndexEntry, SnapshotOmissionFact, SnapshotOmissionReason, SnapshotPath,
    SnapshotSelection, SnapshotSourceIdentity,
};

// W04a capture destination: staging, completion, garbage.
mod capture_complete;
mod capture_dest;
mod capture_dest_gc;
mod capture_dest_private;
mod capture_lock;
mod capture_manifest_read;
mod capture_object_sink;
mod capture_publish;
mod capture_staging;
mod capture_types;

pub use capture_dest::SnapshotArtifacts;
pub use capture_object_sink::ObjectSink;
pub use capture_staging::StagedSnapshot;
pub use capture_types::{CapturedEntry, CapturedSnapshot};

// W04M materialization: verified replay of a published snapshot into a
// private target, with a content-digest receipt.
mod materialize;
mod materialize_types;

pub use materialize::materialize_snapshot;
pub use materialize_types::{MaterializedEntry, MaterializedSnapshot};

// W05c2 reset: return an interrupted Creating target to
// `.git`-pointer-only.
mod materialize_reset;

pub(crate) use materialize_reset::privatize_materialization_target;
pub(crate) use materialize_reset::reset_materialization_target;
