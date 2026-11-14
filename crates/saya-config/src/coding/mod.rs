mod extensions;
pub use extensions::{ExtensionProfile, ExtensionProtocol, ExtensionToolEffect};
mod merge;
mod resolve;

pub(crate) use merge::merge;
pub use resolve::ResolvedCoding;
pub(crate) use resolve::resolve;
