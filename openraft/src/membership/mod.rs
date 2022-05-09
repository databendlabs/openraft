mod effective_membership;
#[allow(clippy::module_inception)] mod membership;

#[cfg(test)] mod membership_test;

pub mod quorum;

pub use effective_membership::EffectiveMembership;
pub use membership::IntoOptionNodes;
pub use membership::Membership;
