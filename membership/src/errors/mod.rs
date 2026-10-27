//! Errors from constructing and editing provided memberships.

mod configs_error;
mod invalid_thresholds;
mod threshold_membership_error;

pub(crate) use configs_error::ConfigsError;
#[since(version = "0.10.0")]
pub use invalid_thresholds::InvalidThresholds;
use openraft_macros::since;
#[since(version = "0.10.0")]
pub use threshold_membership_error::ThresholdMembershipError;

#[since(version = "0.10.0")]
pub use crate::weighted::InvalidWeights;
