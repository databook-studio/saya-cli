//! The shape every run-scoped name must have, shared by roles, endpoints,
//! budget keys, runner programs, and artifacts.

use crate::MAX_NAME_CHARS;

/// True when `name` has the shape a run-scoped name must have: non-empty,
/// bounded, and free of control characters and whitespace. Roles, endpoint
/// names, budget keys, and runner programs all share this shape. Exported so
/// config resolution can reject a malformed `[jobs]` budget key at resolve
/// time with this exact rule, instead of duplicating it.
pub fn is_name_shaped(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// True when `name` is a bare program name the runner can resolve against
/// one program directory: the run-scoped name shape with no path separators
/// and no relative forms. A runner allowlist names programs, never paths —
/// an absolute path or a traversal is a refusal at every layer (config
/// resolution, `RunnerScope`, the tool), because a path-shaped "name" could
/// point anywhere on the filesystem while the sandbox only allows exec
/// inside the program directory. The same rule an artifact name must
/// satisfy, shared so the two cannot drift.
pub fn is_bare_name(name: &str) -> bool {
    is_name_shaped(name)
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
}
