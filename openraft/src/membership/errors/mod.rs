//! Errors for membership validation.

mod unproven_read_write_intersection;
mod write_voter_without_receive_logs;

use openraft_macros::since;
#[since(version = "0.10.0")]
pub use unproven_read_write_intersection::UnprovenReadWriteIntersection;
#[since(version = "0.10.0")]
pub use write_voter_without_receive_logs::WriteVoterWithoutReceiveLogs;
