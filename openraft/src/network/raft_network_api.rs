//! Defines the [`RaftNetworkApi`] super-trait used internally by Openraft.

use crate::RaftTypeConfig;
use crate::network::NetBackoff;
use crate::network::NetSnapshot;
use crate::network::NetStreamAppend;
use crate::network::NetTransferLeader;
use crate::network::NetVote;

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
pub(crate) trait RaftNetworkApi<C>:
    NetBackoff<C> + NetStreamAppend<C> + NetVote<C> + NetSnapshot<C> + NetTransferLeader<C>
where C: RaftTypeConfig
{
}

// Blanket impl: automatically implement RaftNetworkApi for any type
// that implements all required sub-traits.
impl<C, T> RaftNetworkApi<C> for T
where
    C: RaftTypeConfig,
    T: NetBackoff<C> + NetStreamAppend<C> + NetVote<C> + NetSnapshot<C> + NetTransferLeader<C>,
{
}
