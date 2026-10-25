//! Errors from constructing and editing provided memberships.

mod invalid_thresholds;
mod threshold_membership_error;

#[since(version = "0.10.0")]
pub use invalid_thresholds::InvalidThresholds;
use openraft_macros::since;
#[since(version = "0.10.0")]
pub use threshold_membership_error::ThresholdMembershipError;
