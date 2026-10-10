//! Pure authority attenuation for native coding children.

mod admission;
mod authority;
mod values;

pub use authority::{
    AuthorityError, AuthorityRefusal, ChildAuthorityRequest, ChildAuthorityView, ChildRole,
    CommandCheckoutProfile, PlatformSupport, RootDelegationInput, RootDelegationScope,
};
