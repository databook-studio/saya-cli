//! Stable Windows file identity from an open handle.

use std::{fs::File, io, os::windows::io::AsRawHandle};

#[repr(C)]
struct FileTime {
    _low: u32,
    _high: u32,
}

#[repr(C)]
struct ByHandleFileInformation {
    attributes: u32,
    _creation_time: FileTime,
    _last_access_time: FileTime,
    _last_write_time: FileTime,
    volume_serial: u32,
    _file_size_high: u32,
    _file_size_low: u32,
    _links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        file: *mut std::ffi::c_void,
        information: *mut ByHandleFileInformation,
    ) -> i32;
}

/// Returns the volume serial plus file index for the object this handle names.
pub(crate) fn identity(file: &File) -> io::Result<(u64, u64)> {
    let information = information(file)?;
    Ok((
        u64::from(information.volume_serial),
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low),
    ))
}

/// Whether this handle names a reparse point rather than an ordinary file.
pub(crate) fn is_reparse_point(file: &File) -> io::Result<bool> {
    Ok(information(file)?.attributes & 0x0000_0400 != 0)
}

fn information(file: &File) -> io::Result<ByHandleFileInformation> {
    let mut information = std::mem::MaybeUninit::<ByHandleFileInformation>::uninit();
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { information.assume_init() })
}
