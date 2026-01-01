//! Compile-time checks for deprecated or removed feature flags.
//!
//! This module emits helpful compile-time errors with migration guidance
//! when users enable feature flags that have been removed or renamed.

#[cfg(feature = "loosen-follower-log-revert")]
compile_error!(
    "The feature flag `loosen-follower-log-revert` is removed since `0.10.0`. \
     Use `Config::allow_log_reversion` instead."
);

#[cfg(feature = "singlethreaded")]
compile_error!(
    "The feature flag `singlethreaded` is renamed to `single-threaded` since `0.10.0`. \
     Please update your Cargo.toml to use `single-threaded` instead."
);

#[cfg(feature = "adapt-network-v1")]
compile_error!(
    "The feature flag `adapt-network-v1` is removed since `0.10.0`. \
     For backward compatibility with the v1 `RaftNetwork` trait and chunk-based snapshot transport, \
     use the `openraft-legacy` crate instead."
);

#[cfg(feature = "single-term-leader")]
compile_error!(
    r#"`single-term-leader` is removed since `0.10.0`.
To enable standard Raft mode:
- either add `LeaderId = openraft::impls::leader_id_std::LeaderId` to `declare_raft_types!(YourTypeConfig)` statement,
- or add `type LeaderId: openraft::impls::leader_id_std::LeaderId` to the `RaftTypeConfig` implementation."#
);
