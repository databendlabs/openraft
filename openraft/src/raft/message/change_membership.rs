use std::fmt;

use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::ClientWriteResponse;

/// Responses from the physical log entries of a membership change.
#[since(version = "0.10.0", change = "added payload-aware membership outcome")]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "C::R: crate::AppDataResponse")
)]
pub struct ChangeMembershipOutcome<C>
where C: RaftTypeConfig
{
    /// The response from the requested membership change.
    pub first: ClientWriteResponse<C>,

    /// The response from flattening a joint membership to a uniform membership.
    pub uniform: Option<ClientWriteResponse<C>>,
}

impl<C> fmt::Debug for ChangeMembershipOutcome<C>
where
    C: RaftTypeConfig,
    C::R: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChangeMembershipOutcome")
            .field("first", &self.first)
            .field("uniform", &self.uniform)
            .finish()
    }
}
