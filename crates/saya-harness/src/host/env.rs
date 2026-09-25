//! The host child's environment: built, not inherited. The child
//! receives PATH, HOME, TMPDIR — the parent's values, read once when the
//! config applies — plus whatever the caller explicitly passes. Nothing
//! else reaches the child.

use super::resolve::HostError;

/// One explicitly passed variable, validated at insert: a NAME=value name
/// the child's environment can represent (ASCII letters, digits,
/// underscores, not starting with a digit).
#[derive(Debug, Clone, Default)]
pub struct HostEnv {
    vars: Vec<(String, String)>,
}

impl HostEnv {
    /// Inserts one explicitly passed variable. A malformed name is a typed
    /// refusal; values are unconstrained bytes the caller chose.
    pub fn insert(&mut self, name: String, value: String) -> Result<(), HostError> {
        if !is_env_name(&name) {
            return Err(HostError::EnvNameInvalid { name });
        }
        if let Some(slot) = self.vars.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = value;
        } else {
            self.vars.push((name, value));
        }
        Ok(())
    }

    fn iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.vars.iter()
    }
}

/// Builds the child's full environment: PATH (the value resolution
/// searched), HOME and TMPDIR (the parent's values, when set), then the
/// caller's explicitly passed variables, which win on collision. The order
/// is fixed so a caller-passed PATH overrides the searched one only by
/// saying so.
pub fn build_env(path: &str, extra: &HostEnv) -> Vec<(String, String)> {
    let mut built: Vec<(String, String)> = Vec::with_capacity(3 + extra.vars.len());
    built.push(("PATH".to_owned(), path.to_owned()));
    // HOME and TMPDIR carry the parent's values, read here once at apply
    // time; when unset they are simply absent from the child.
    for name in ["HOME", "TMPDIR"] {
        if let Ok(value) = std::env::var(name) {
            built.push((name.to_owned(), value));
        }
    }
    for (name, value) in extra.iter() {
        if let Some(slot) = built.iter_mut().find(|(n, _)| *n == *name) {
            slot.1.clone_from(value);
        } else {
            built.push((name.clone(), value.clone()));
        }
    }
    built
}

/// True when `name` can appear on the left of `NAME=value`: non-empty,
/// ASCII letters/digits/underscores, not starting with a digit.
fn is_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
