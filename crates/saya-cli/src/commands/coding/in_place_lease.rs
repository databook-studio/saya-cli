//! The one-live-job lease over an in-place root.
//!
//! An in-place job writes the user's own folder, so two live jobs over the
//! same root would interleave edits and before-images. The lease is a
//! non-blocking `flock` on a private per-root lock file, held by the job for
//! its lifetime; the descriptor is close-on-exec so a forked child never
//! inherits the lock. Read verbs never take it.

use std::path::Path;
use std::sync::Arc;

use saya_harness::coding::local::JobBaseline;
use saya_harness::coding::local::before_image::BeforeImageStore;

use super::CodingError;

/// The parts an in-place job carries for edit-time recovery: its shared
/// before-image store and, when `take_lease` is set, the root lease. A
/// managed baseline needs neither; a lifecycle verb keeps the store but
/// never takes the lease.
pub(super) fn job_parts(
    plan: &JobBaseline,
    records: &Path,
    state_root: &Path,
    take_lease: bool,
) -> Result<(Option<Arc<BeforeImageStore>>, Option<InPlaceLease>), CodingError> {
    match plan {
        JobBaseline::InPlace(baseline) => {
            let store = BeforeImageStore::open(records).map_err(CodingError::owner)?;
            let lease = take_lease
                .then(|| InPlaceLease::acquire(state_root, baseline.repository_id().as_str()))
                .transpose()?;
            Ok((Some(Arc::new(store)), lease))
        }
        JobBaseline::Git(_) => Ok((None, None)),
    }
}

/// One held in-place root lease. Dropping it closes the descriptor and
/// releases the lock.
pub(super) struct InPlaceLease {
    #[cfg(unix)]
    _lock: std::os::fd::OwnedFd,
}

impl InPlaceLease {
    /// Takes the lease for `root_id` under `<state_root>/inplace-leases/`.
    /// A second live job over the same root gets [`CodingError::InPlaceRootBusy`].
    pub(super) fn acquire(state_root: &Path, root_id: &str) -> Result<Self, CodingError> {
        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::OpenOptionsExt;

            let directory = super::host::private_directory(&state_root.join("inplace-leases"))?;
            let path = directory.join(format!("{root_id}.lock"));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true);
            options
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
            let file = options.open(&path).map_err(CodingError::owner)?;
            // SAFETY: `file` owns a valid descriptor for the duration of the call.
            let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if locked != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::WouldBlock {
                    return Err(CodingError::InPlaceRootBusy);
                }
                return Err(CodingError::owner(error));
            }
            Ok(Self { _lock: file.into() })
        }
        #[cfg(not(unix))]
        {
            let _ = (state_root, root_id);
            Err(CodingError::owner(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this host cannot lock an in-place folder",
            )))
        }
    }
}
