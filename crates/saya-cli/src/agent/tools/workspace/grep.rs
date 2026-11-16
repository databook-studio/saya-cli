#[cfg(test)]
pub(crate) const WORKSPACE_GREP_MAX_MATCHES: usize = 500;
#[cfg(test)]
pub(crate) const WORKSPACE_GREP_MAX_LINE_BYTES: usize = 2_000;
#[cfg(test)]
pub(crate) const WORKSPACE_GREP_MAX_FILE_BYTES: u64 =
    saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES;
