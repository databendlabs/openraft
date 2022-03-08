#[allow(clippy::module_inception)]
mod membership;

#[cfg(test)]
mod membership_test;

pub mod quorum;

pub use membership::IntoOptionNodes;
pub use membership::Membership;
