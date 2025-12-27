//! Deprecated RaftNetwork trait stub.
//!
//! This module provides a deprecated trait directing users to use
//! the `openraft-network-v1` crate for the v1 network API.

use crate::RaftTypeConfig;

/// **REMOVED**: Use `openraft_network_v1::RaftNetwork` instead.
///
/// The `RaftNetwork` trait has been moved to the `openraft-network-v1` crate.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-network-v1 = "0.10"
/// ```
///
/// Update imports:
/// ```ignore
/// use openraft_network_v1::RaftNetwork;
/// ```
#[deprecated(
    since = "0.10.0",
    note = "RaftNetwork has been moved to the `openraft-network-v1` crate. \
            Add `openraft-network-v1` to your dependencies and use \
            `openraft_network_v1::RaftNetwork` instead."
)]
pub trait RaftNetwork<C: RaftTypeConfig> {}
