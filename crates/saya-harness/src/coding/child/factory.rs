use std::{error::Error, fmt};

use saya_agent::{ApprovalDecider, CancellationToken, ChatProvider, ToolDefinition, ToolExecutor};
use saya_types::CodingFailureClass;
use serde::{Deserialize, Serialize};

use super::spec::ChildExecutionSpec;

/// Synchronous composition boundary for one already-narrowed child.
pub trait ChildFactory: Send + Sync {
    /// Builds the provider, scoped decider and matching toolset from this immutable view.
    /// `cancellation` is the token used by this invocation's agent loop; tools
    /// that own processes must bind their lifetime to this same token.
    fn build(
        &self,
        spec: &ChildExecutionSpec,
        cancellation: &CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError>;

    /// Native owners may retain one safe, payload-free failure summary after
    /// mandatory tool settlement. Generic factories have no such owner.
    fn owner_failure(&self) -> Option<OwnerFailureSnapshot> {
        None
    }
}

/// The bounded terminal cause state supplied by a native child owner. It is
/// intentionally free of text, paths, protocol payloads, and authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerFailureSnapshot {
    primary: CodingFailureClass,
    secondary: Option<CodingFailureClass>,
}

impl OwnerFailureSnapshot {
    pub fn new(primary: CodingFailureClass, secondary: Option<CodingFailureClass>) -> Self {
        Self {
            primary,
            secondary: secondary.filter(|secondary| *secondary != primary),
        }
    }

    pub fn primary(self) -> CodingFailureClass {
        self.primary
    }

    pub fn secondary(self) -> Option<CodingFailureClass> {
        self.secondary
    }
}

/// Owned loop dependencies whose tool list was built from the same child view.
pub struct ChildComponents {
    pub(crate) provider: Box<dyn ChatProvider>,
    pub(crate) tools: Box<dyn ToolExecutor>,
    pub(crate) definitions: Vec<ToolDefinition>,
    pub(crate) approval: Box<dyn ApprovalDecider>,
}

impl ChildComponents {
    /// Groups the exact tools and definitions prepared from one authority view.
    pub fn new(
        provider: Box<dyn ChatProvider>,
        tools: Box<dyn ToolExecutor>,
        definitions: Vec<ToolDefinition>,
        approval: Box<dyn ApprovalDecider>,
    ) -> Self {
        Self {
            provider,
            tools,
            definitions,
            approval,
        }
    }

    /// The child's executor, as composed from its authority view. A caller
    /// that decorates the dispatch path (a hook gate, an observer) needs the
    /// exact executor the loop will call, not a second one built beside it.
    pub fn tools(&self) -> &dyn ToolExecutor {
        self.tools.as_ref()
    }

    /// The definitions the child's loop advertises, as composed from the same
    /// authority view — including a member the checkout's own state decided
    /// rather than the view, so a caller can state what the child was offered
    /// without rebuilding the composition.
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }
}

/// A factory failure that keeps its source available to the trusted caller.
pub struct ChildFactoryError(Box<dyn Error + Send + Sync>);

impl ChildFactoryError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(source))
    }
}

impl fmt::Debug for ChildFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildFactoryError { .. }")
    }
}

impl fmt::Display for ChildFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("child components could not be prepared")
    }
}

impl Error for ChildFactoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GenericFactory;

    impl ChildFactory for GenericFactory {
        fn build(
            &self,
            _: &ChildExecutionSpec,
            _: &CancellationToken,
        ) -> Result<ChildComponents, ChildFactoryError> {
            unreachable!("the default owner-failure contract does not compose a child")
        }
    }

    #[test]
    fn generic_factories_have_no_native_owner_failure() {
        assert_eq!(GenericFactory.owner_failure(), None);
    }

    #[test]
    fn snapshot_keeps_only_two_safe_classes() {
        let snapshot = OwnerFailureSnapshot::new(
            CodingFailureClass::ExtensionExchange,
            Some(CodingFailureClass::MandatoryPersistence),
        );
        assert_eq!(snapshot.primary(), CodingFailureClass::ExtensionExchange);
        assert_eq!(
            snapshot.secondary(),
            Some(CodingFailureClass::MandatoryPersistence)
        );
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::json!({
                "primary":"extension_exchange",
                "secondary":"mandatory_persistence"
            })
        );
    }

    #[test]
    fn duplicate_secondary_is_not_retained() {
        let snapshot = OwnerFailureSnapshot::new(
            CodingFailureClass::ExtensionExchange,
            Some(CodingFailureClass::ExtensionExchange),
        );
        assert_eq!(snapshot.secondary(), None);
    }
}
