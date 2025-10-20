//! Log entry types and traits.
//!
//! This module defines types for Raft log entries that carry application data and membership
//! changes.
//!
//! ## Key Types
//!
//! - [`Entry`] - Default log entry implementation containing log ID and payload
//! - [`EntryPayload`] - Payload types: application data, membership config, or blank
//! - [`RaftEntry`] - Trait that log entries must implement
//! - [`RaftPayload`] - Trait for entry payload types
//!
//! ## Overview
//!
//! Each log entry contains:
//! - **Log ID**: `(term, node_id, index)` uniquely identifying the entry
//! - **Payload**: Either application data, membership change, or blank (noop)
//!
//! ## Entry Types
//!
//! - **Application entries**: Carry user data (`D` in [`RaftTypeConfig`])
//! - **Membership entries**: Record cluster configuration changes
//! - **Blank entries**: No-op entries appended by new leaders
//!
//! Applications typically use the default [`Entry`] type, or implement [`RaftEntry`] for custom
//! behavior.

#[cfg(doc)]
use crate::RaftTypeConfig;

#[allow(clippy::module_inception)]
mod entry;
pub mod payload;
mod raft_entry;
pub(crate) mod raft_entry_ext;
mod raft_payload;

pub use entry::Entry;
pub use payload::EntryPayload;
pub use raft_entry::RaftEntry;
pub use raft_payload::RaftPayload;
