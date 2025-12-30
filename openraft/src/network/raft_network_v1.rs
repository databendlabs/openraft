//! Deprecated RaftNetwork trait stub.
//!
//! This module provides a deprecated trait directing users to use
//! the `openraft-legacy` crate for the v1 network API.

use crate::RaftTypeConfig;

/// **REMOVED**: Use `openraft_legacy::RaftNetwork` instead.
///
/// The `RaftNetwork` trait has been moved to the `openraft-legacy` crate.
///
/// # Migration
///
/// Add to `Cargo.toml`:
/// ```toml
/// [dependencies]
/// openraft-legacy = "0.10"
/// ```
///
/// Update imports:
/// ```ignore
/// use openraft_legacy::network_v1::RaftNetwork;
/// ```
#[deprecated(
    since = "0.10.0",
    note = "RaftNetwork has been moved to the `openraft-legacy` crate. \
            Add `openraft-legacy` to your dependencies and use \
            `openraft_legacy::RaftNetwork` instead."
)]
pub trait RaftNetwork<C: RaftTypeConfig> {}
