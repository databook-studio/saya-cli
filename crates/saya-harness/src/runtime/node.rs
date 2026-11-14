//! The one compiled Node/npm identity this installer may publish.

/// The sole OCI image repository the managed runtime accepts.
pub const NODE_IMAGE_REPOSITORY: &str = "docker.io/library/node";
/// The human-readable tag that selected the immutable manifest.
pub const NODE_IMAGE_TAG: &str = "24.21.0-bookworm-slim";
/// The Linux ARM64 manifest digest selected from the official Node index.
pub const NODE_IMAGE_MANIFEST_DIGEST: &str =
    "sha256:ba6b7d0ee561d9d49deb99e2b93c98632cc287b6cca1a6b18d66127054efa4dc";
/// The canonical reference used for pull and inspection.
pub const NODE_IMAGE_REFERENCE: &str = "docker.io/library/node@sha256:ba6b7d0ee561d9d49deb99e2b93c98632cc287b6cca1a6b18d66127054efa4dc";
/// The only Node version a ready record may claim.
pub const NODE_VERSION: &str = "v24.21.0";
/// The only bundled npm version a ready record may claim.
pub const NPM_VERSION: &str = "11.19.0";

/// The immutable image identity required for the managed Node runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeImageReference;

impl NodeImageReference {
    /// Returns the registry repository, never a caller-provided location.
    pub const fn repository(self) -> &'static str {
        NODE_IMAGE_REPOSITORY
    }
    /// Returns the selection tag retained for receipts and diagnostics.
    pub const fn tag(self) -> &'static str {
        NODE_IMAGE_TAG
    }
    /// Returns the required immutable Linux ARM64 manifest digest.
    pub const fn manifest_digest(self) -> &'static str {
        NODE_IMAGE_MANIFEST_DIGEST
    }
    /// Returns the exact digest-qualified pull reference.
    pub const fn reference(self) -> &'static str {
        NODE_IMAGE_REFERENCE
    }
}

/// The sole compiled image selection. No caller can replace it.
pub const PINNED_NODE_IMAGE: NodeImageReference = NodeImageReference;

/// Explicit consent required before image acquisition can begin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeInstallConsent {
    Granted,
}

/// A successful installation or revalidation receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeInstallReceipt {
    /// A verified image was pulled and a new private record was published.
    Installed { image: NodeImageReference },
    /// An existing private record was revalidated without replacing it.
    Reused { image: NodeImageReference },
}

impl NodeInstallReceipt {
    /// Returns the compiled image proven for this successful operation.
    pub const fn image(&self) -> NodeImageReference {
        match self {
            Self::Installed { image } | Self::Reused { image } => *image,
        }
    }
    /// Returns the exact Node version proven by the bounded container probe.
    pub const fn node_version(&self) -> &'static str {
        NODE_VERSION
    }
    /// Returns the exact npm version proven by the bounded container probe.
    pub const fn npm_version(&self) -> &'static str {
        NPM_VERSION
    }
}
