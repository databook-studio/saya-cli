//! The resolved `[tools]` setting: the validated allow list, or `None` when no
//! layer stated one.

use super::{ToolsAllow, ToolsAllowError};
use crate::{ConfigError, ToolsFile};

/// The effective `[tools]` setting. `allow: None` is the default and means
/// "narrow nothing": every surface advertises exactly what it composes, which
/// is what it advertised before this setting existed. It is deliberately not
/// an empty set — an empty set would advertise no tool at all, and stating one
/// is a typed error ([`super::ToolsAllowError::Empty`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedTools {
    pub allow: Option<ToolsAllow>,
}

impl ResolvedTools {
    /// Whether `name` survives the list. `None` admits everything — the list
    /// only ever narrows, so an unstated list is the widest surface, not a
    /// grant of anything.
    pub fn admits(&self, name: &str) -> bool {
        self.allow.as_ref().is_none_or(|allow| allow.allows(name))
    }

    /// Whether the user explicitly stated `name` in the list. Distinct from
    /// [`Self::admits`], which is also true when no list was stated: a
    /// surface uses this to advertise a tool it suppresses by default when the
    /// user named it, so `[tools] allow = ["list"]` opts back in.
    pub fn states(&self, name: &str) -> bool {
        self.allow.as_ref().is_some_and(|allow| allow.allows(name))
    }

    /// The set a helper child runs under: the job's list minus `helpers`,
    /// which only the root may call — a helper cannot delegate. An unstated
    /// list stays unstated: the helper's own composition is what narrows it.
    pub fn for_helper(&self) -> Self {
        Self {
            allow: self.allow.as_ref().map(|allow| allow.without("helpers")),
        }
    }

    /// The set a *stated* list resolves to, for the callers that hold raw
    /// words rather than a config layer — a persisted coding request, or the
    /// `--tools` words on a coding start. `None` is the default set; a stated
    /// list is validated exactly as `[tools] allow` is, so a request and a
    /// config layer can never disagree about what a name means.
    pub fn from_stated(raw: Option<&[String]>) -> Result<Self, ToolsAllowError> {
        match raw {
            None => Ok(Self::default()),
            Some(raw) => Ok(Self {
                allow: Some(ToolsAllow::parse(raw)?),
            }),
        }
    }

    /// This set in the form [`Self::from_stated`] reads back: the canonical
    /// names, or `None` when nothing was stated. A job persists exactly what it
    /// resolved, so a resume resolves the same set.
    pub fn stated(&self) -> Option<Vec<String>> {
        self.allow
            .as_ref()
            .map(|allow| allow.names().map(str::to_owned).collect())
    }
}

pub(crate) fn resolve(file: &ToolsFile) -> Result<ResolvedTools, ConfigError> {
    let Some(raw) = file.allow.as_ref() else {
        return Ok(ResolvedTools::default());
    };
    Ok(ResolvedTools {
        allow: Some(ToolsAllow::parse(raw).map_err(ConfigError::ToolsAllow)?),
    })
}
