//! Pure authority attenuation for native coding children.

mod admission;
mod approval;
mod authority;
mod values;

pub use approval::ChildApprovalDecider;
pub use authority::{
    AuthorityError, AuthorityRefusal, ChildAuthorityRequest, ChildAuthorityView, ChildRole,
    CommandCheckoutProfile, PlatformSupport, RootDelegationInput, RootDelegationScope,
};
