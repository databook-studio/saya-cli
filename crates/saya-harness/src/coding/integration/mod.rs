//! Integration receipts: shared assigned-path contracts.

mod paths;

pub use paths::{
    AssignedPaths, AssignedPathsError, MAX_ASSIGNED_PATH_BYTES, MAX_ASSIGNED_PATHS,
    MAX_ASSIGNED_PATHS_TOTAL_BYTES, casefold_collision,
};
