//! The one-live-job lease over an in-place root.
//!
//! An in-place job writes the user's own folder, so two unrelated live jobs over
//! the same root would interleave edits and before-images. The lease is a
//! non-blocking `flock` on a private per-root lock file, held by the job for
//! its lifetime; the descriptor is close-on-exec so a forked child never
//! inherits the lock. Read verbs never take it.
//!
//! A session's own run is the one owner whose children are expected: the lease
//! names the session, and a job that session opens over the same folder is
//! admitted as its dependent — the way a subagent works in its parent's working
//! directory. A dependent holds no second descriptor; releasing it closes
//! nothing, and the session's run keeps the lock until it ends. Every other
//! caller gets today's refusal.

use std::path::Path;
use std::sync::Arc;

use saya_harness::coding::local::JobBaseline;
use saya_harness::coding::local::before_image::BeforeImageStore;

use super::CodingError;

/// The parts an in-place job carries for edit-time recovery: its shared
/// before-image store and, when `take_lease` is set, the root lease. A
/// managed baseline needs neither; a lifecycle verb keeps the store but
/// never takes the lease.
///
/// `owner` is the name the lease states: a session's run names its session, and
/// a job that states that same `parent` is admitted as its dependent.
pub(super) fn job_parts(
    plan: &JobBaseline,
    records: &Path,
    state_root: &Path,
    take_lease: bool,
    owner: &str,
    parent: Option<&str>,
) -> Result<(Option<Arc<BeforeImageStore>>, Option<InPlaceLease>), CodingError> {
    match plan {
        JobBaseline::InPlace(baseline) => {
            let store = BeforeImageStore::open(records).map_err(CodingError::owner)?;
            let lease = take_lease
                .then(|| {
                    InPlaceLease::acquire(
                        state_root,
                        baseline.repository_id().as_str(),
                        owner,
                        parent,
                    )
                })
                .transpose()?;
            Ok((Some(Arc::new(store)), lease))
        }
        JobBaseline::Git(_) => Ok((None, None)),
    }
}

/// One held in-place root lease. Dropping it closes the descriptor and
/// releases the lock — unless it is the session's dependent, whose own drop
/// closes nothing.
pub(super) struct InPlaceLease {
    /// The root this lease is over, so the process-local record of what it
    /// holds names the same folder the lock file does.
    root_id: String,
    /// Whether this lease is the one that holds the lock. A dependent's is not:
    /// the session's run keeps the folder until it ends.
    holds_lock: bool,
    #[cfg(unix)]
    _lock: Option<std::os::fd::OwnedFd>,
    #[cfg(windows)]
    _lock: Option<std::fs::File>,
}

impl Drop for InPlaceLease {
    fn drop(&mut self) {
        if self.holds_lock {
            super::live_runs::released(&self.root_id);
        }
    }
}

impl InPlaceLease {
    /// Takes the lease for `root_id` under `<state_root>/inplace-leases/`, and
    /// records it as this process's own under `owner`.
    ///
    /// A second live job over the same root gets [`CodingError::InPlaceRootBusy`]
    /// — unless it states the `parent` this process already leases the root for,
    /// in which case it is that session's dependent and takes no lock of its own.
    pub(super) fn acquire(
        state_root: &Path,
        root_id: &str,
        owner: &str,
        parent: Option<&str>,
    ) -> Result<Self, CodingError> {
        match super::live_runs::lease_owner(root_id) {
            Some(lease_owner) if parent == Some(lease_owner.as_str()) => {
                Ok(Self::dependent(root_id))
            }
            Some(_) => Err(CodingError::InPlaceRootBusy),
            None => {
                let lease = Self::lock(state_root, root_id)?;
                super::live_runs::leased(&lease.root_id, owner);
                Ok(lease)
            }
        }
    }

    /// One dependent lease: admitted beside the session's run, holding no lock
    /// and releasing nothing.
    fn dependent(root_id: &str) -> Self {
        Self {
            root_id: root_id.to_owned(),
            holds_lock: false,
            _lock: None,
        }
    }

    /// The lock itself.
    fn lock(state_root: &Path, root_id: &str) -> Result<Self, CodingError> {
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
            Ok(Self {
                root_id: root_id.to_owned(),
                holds_lock: true,
                _lock: Some(file.into()),
            })
        }
        #[cfg(windows)]
        {
            use std::fs::TryLockError;
            use std::os::windows::fs::OpenOptionsExt as _;

            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            let directory = super::host::private_directory(&state_root.join("inplace-leases"))?;
            let path = directory.join(format!("{root_id}.lock"));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&path)
                .map_err(CodingError::owner)?;
            // Windows byte-range locks are owned by the handle, so a second
            // live job over the same root gets the same busy refusal as the
            // unix `flock`. A reparse point in the lock file's place is
            // refused rather than locked.
            if saya_harness::workspace::windows_identity::is_reparse_point(&file)
                .map_err(CodingError::owner)?
            {
                return Err(CodingError::StateDirectory);
            }
            match file.try_lock() {
                Ok(()) => Ok(Self {
                    root_id: root_id.to_owned(),
                    holds_lock: true,
                    _lock: Some(file),
                }),
                Err(TryLockError::WouldBlock) => Err(CodingError::InPlaceRootBusy),
                Err(TryLockError::Error(error)) => Err(CodingError::owner(error)),
            }
        }
        #[cfg(all(not(unix), not(windows)))]
        {
            let _ = (state_root, root_id);
            Err(CodingError::owner(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this host cannot lock an in-place folder",
            )))
        }
    }
}

/// Whether `root_id`'s lease is held by a live owner, read-only: one
/// non-blocking attempt to take it, given straight back. Nothing is written and
/// no owner is displaced — the attempt is how the operating system is asked
/// whether the lock is held. A lock file no process holds, a dead owner's, and a
/// missing one both read as free, so an ended run keeps today's recovery
/// behaviour.
pub(crate) fn leased(state_root: &Path, root_id: &str) -> Result<bool, CodingError> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;

        let path = state_root
            .join("inplace-leases")
            .join(format!("{root_id}.lock"));
        let mut options = OpenOptions::new();
        // No `create`: a lease file only its owner makes is never written here.
        options
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(CodingError::owner(error)),
        };
        // SAFETY: `file` owns a valid descriptor for the duration of the call.
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked == 0 {
            // Taken only to ask the question; dropping `file` gives it back.
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(true);
        }
        Err(CodingError::owner(error))
    }
    #[cfg(windows)]
    {
        use std::fs::TryLockError;
        use std::os::windows::fs::OpenOptionsExt as _;

        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        let path = state_root
            .join("inplace-leases")
            .join(format!("{root_id}.lock"));
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(CodingError::owner(error)),
        };
        match file.try_lock() {
            Ok(()) => Ok(false),
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(error)) => Err(CodingError::owner(error)),
        }
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = (state_root, root_id);
        Ok(false)
    }
}
