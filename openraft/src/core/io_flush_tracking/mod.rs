//! I/O flush progress tracking.
//!
//! This module provides watch-based notification channels for tracking when Raft I/O operations
//! (vote saves and log appends) are flushed to storage. It enables applications to:
//!
//! - Wait for specific log entries to be durably written
//! - Track vote changes across leader elections
//! - Ensure data persistence before responding to clients
//!
//! The tracking is based on monotonically increasing [`IOId`] values that identify each I/O
//! operation. When storage completes an operation, it notifies RaftCore, which updates the
//! progress channels.

mod flush_point;
mod watch_progress;
mod watcher;

pub use flush_point::FlushPoint;
pub use watch_progress::LogProgress;
pub use watch_progress::VoteProgress;
pub(crate) use watcher::IoProgressSender;
pub(crate) use watcher::IoProgressWatcher;
