use crate::{CodingFile, ConfigFile};

pub(super) fn snapshot(file: &ConfigFile) -> CodingFile {
    file.coding.clone()
}

pub(super) fn revert(file: &mut ConfigFile, trusted: &CodingFile) -> bool {
    if file.coding == *trusted {
        false
    } else {
        file.coding = trusted.clone();
        true
    }
}
