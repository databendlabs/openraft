//! Errors for membership validation.

mod membership_error;
mod unproven_read_write_intersection;
mod write_voter_without_receive_logs;

#[since(version = "0.10.0")]
pub use membership_error::MembershipError;
use openraft_macros::since;
#[since(version = "0.10.0")]
pub use unproven_read_write_intersection::UnprovenReadWriteIntersection;
#[since(version = "0.10.0")]
pub use write_voter_without_receive_logs::WriteVoterWithoutReceiveLogs;
