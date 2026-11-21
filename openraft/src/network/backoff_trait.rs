//! Defines the [`NetBackoff`] trait for network backoff behavior.

use openraft_macros::since;

use crate::OptionalSend;
use crate::OptionalSync;
use crate::RaftTypeConfig;
use crate::network::Backoff;

/// Provides backoff strategy for network operations.
///
/// **For most applications, implement [`RaftNetworkV2`] instead.** This trait is
/// automatically derived from `RaftNetworkV2` via blanket implementation.
///
/// Direct implementation is an advanced option for fine-grained control over
/// retry behavior when nodes are unreachable.
///
/// [`RaftNetworkV2`]: crate::network::RaftNetworkV2
pub trait NetBackoff<C>: OptionalSend + OptionalSync + 'static
where C: RaftTypeConfig
{
    /// Build a backoff instance if the target node is temporarily(or permanently) unreachable.
    ///
    /// When a [`Unreachable`](`crate::error::Unreachable`) error is returned from the `Network`
    /// methods, Openraft does not retry connecting to a node immediately. Instead, it sleeps
    /// for a while and retries. The duration of the sleep is determined by the backoff
    /// instance.
    ///
    /// The backoff returns the ith sleep interval before the ith retry. Make it infinite: after
    /// it ends, Openraft keeps retrying with a fixed delay, as [`Backoff`] describes. The
    /// returned instance will be dropped if a successful RPC is made.
    ///
    /// Return `None` to use the default backoff configured in
    /// [`Config::backoff`](crate::Config::backoff). Only this `None` selects that policy: a
    /// returned backoff that runs out does not fall back to it.
    #[since(
        version = "0.10.0",
        change = "changed return type to Option<Backoff>; None delegates to Config::backoff"
    )]
    fn backoff(&self) -> Option<Backoff>;
}
