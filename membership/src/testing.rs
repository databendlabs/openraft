//! Membership helpers and checks shared by unit tests.

use openraft::membership::NodeRole;

/// Returns the role of a voter whose config sets no role for it.
pub(crate) fn voter_role() -> NodeRole {
    NodeRole::Campaigner | NodeRole::ReceiveLogs
}
