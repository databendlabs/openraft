//! Raft configuration and runtime settings.
//!
//! This module defines configuration types for controlling Raft behavior.
//!
//! ## Key Types
//!
//! - [`Config`] - Main configuration for Raft runtime behavior
//! - [`SnapshotPolicy`] - Policy for triggering automatic snapshots
//! - [`RuntimeConfig`] - Dynamic configuration that can be changed at runtime
//! - [`ConfigError`] - Configuration validation errors
//!
//! ## Overview
//!
//! [`Config`] controls key aspects of Raft operation:
//! - **Election timeouts**: Min/max duration before starting election
//! - **Heartbeat interval**: Frequency of leader heartbeats to followers
//! - **Replication settings**: Max entries per payload, lag thresholds
//! - **Snapshot policy**: When to trigger automatic snapshots
//! - **Log management**: Purge batch size, max logs to keep
//!
//! ## Usage
//!
//! ```ignore
//! use openraft::Config;
//!
//! let config = Config {
//!     heartbeat_interval: 50,
//!     election_timeout_min: 150,
//!     election_timeout_max: 300,
//!     ..Default::default()
//! };
//!
//! let config = config.validate()?;
//! ```
//!
//! See [`Config`] for all available options and their defaults.

#[allow(clippy::module_inception)]
mod config;
mod error;

#[cfg(test)]
mod config_test;

pub use config::Config;
pub(crate) use config::RuntimeConfig;
pub use config::SnapshotPolicy;
pub use error::ConfigError;
