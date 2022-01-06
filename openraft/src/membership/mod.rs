#[allow(clippy::module_inception)]
mod membership;

#[cfg(test)]
mod membership_test;

pub use membership::Membership;
