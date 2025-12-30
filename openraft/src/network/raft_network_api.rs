//! Defines the [`RaftNetworkApi`] super-trait used internally by Openraft.

use crate::RaftTypeConfig;
use crate::network::RaftNetworkBackoff;
use crate::network::RaftNetworkSnapshot;
use crate::network::RaftNetworkStreamAppend;
use crate::network::RaftNetworkTransferLeader;
use crate::network::RaftNetworkVote;

/// The complete network API required by Openraft.
///
/// This trait combines all network sub-traits into a single interface that Openraft
/// uses internally. It is automatically implemented for any type that implements
/// all the required sub-traits.
///
/// Users typically do NOT implement this trait directly. Instead, they either:
/// 1. Implement [`RaftNetworkV2`] - all sub-traits are auto-implemented via blanket impl
/// 2. Implement individual sub-traits directly - `RaftNetworkApi` is auto-implemented
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
pub trait RaftNetworkApi<C>:
    RaftNetworkBackoff<C>
    + RaftNetworkStreamAppend<C>
    + RaftNetworkVote<C>
    + RaftNetworkSnapshot<C>
    + RaftNetworkTransferLeader<C>
where C: RaftTypeConfig
{
}

// Blanket impl: automatically implement RaftNetworkApi for any type
// that implements all required sub-traits.
impl<C, T> RaftNetworkApi<C> for T
where
    C: RaftTypeConfig,
    T: RaftNetworkBackoff<C>
        + RaftNetworkStreamAppend<C>
        + RaftNetworkVote<C>
        + RaftNetworkSnapshot<C>
        + RaftNetworkTransferLeader<C>,
{
}
