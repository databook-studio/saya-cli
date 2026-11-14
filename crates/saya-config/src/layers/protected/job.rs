use crate::{ConfigFile, JobFile};

pub(super) fn snapshot(file: &ConfigFile) -> JobFile {
    file.job.clone()
}

pub(super) fn revert(file: &mut ConfigFile, trusted: &JobFile) -> bool {
    if file.job == *trusted {
        false
    } else {
        file.job = trusted.clone();
        true
    }
}
