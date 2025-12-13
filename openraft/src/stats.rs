//! Runtime statistics for monitoring Raft operations.
//!
//! This module provides types for tracking runtime statistics such as
//! batch sizes for apply and append operations.
//!
//! # Example
//!
//! ```ignore
//! let raft = Raft::new(...).await?;
//! let stats = raft.runtime_stats();
//! stats.with_mut(|s| {
//!     println!("{}", s.display());
//! });
//! ```

pub use crate::base::histogram::Histogram;
pub use crate::base::histogram::PercentileStats;
pub use crate::core::RuntimeStats;
pub use crate::core::RuntimeStatsDisplay;
pub use crate::core::SharedRuntimeState;
