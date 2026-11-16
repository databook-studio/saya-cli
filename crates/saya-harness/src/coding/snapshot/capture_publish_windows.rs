//! Windows publication renames: `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`
//! so the rename is flushed to disk, and without `MOVEFILE_REPLACE_EXISTING`
//! for the publish boundary so an existing final name is refused by the
//! filesystem rather than replaced. Object promotion uses the replace form:
//! the bytes are verified against the digest that names them, so replacing
//! identical content is a no-op by construction.

use std::{io, os::windows::ffi::OsStrExt as _, path::Path};

const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
}

/// Renames `from` to `to`, refusing rather than replacing any existing entry.
pub(super) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_WRITE_THROUGH)
}

/// Renames `from` over `to`, replacing an existing entry.
pub(super) fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
}

fn move_file(from: &Path, to: &Path, flags: u32) -> io::Result<()> {
    let from = wide(from)?;
    let to = wide(to)?;
    let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"));
    }
    units.push(0);
    Ok(units)
}
