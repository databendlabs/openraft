//! Runtime statistics for monitoring Raft operations.
//!
//! This module provides types for tracking runtime statistics such as
//! batch sizes for apply and append operations.
//!
//! # Example
//!
//! ```ignore
//! let raft = Raft::new(...).await?;
//! let stats = raft.runtime_stats().await?;
//! println!("{}", stats.display());
//! ```

pub use crate::base::histogram::Histogram;
pub use crate::base::histogram::PercentileStats;
#[cfg(feature = "runtime-stats")]
pub use crate::base::range_values::RangeValues;
pub use crate::core::NotificationName;
pub use crate::core::RuntimeStats;
pub use crate::core::RuntimeStatsDisplay;
#[cfg(feature = "runtime-stats")]
pub use crate::core::log_stage::LogStageHistograms;
pub use crate::core::raft_msg::ExternalCommandName;
pub use crate::core::raft_msg::RaftMsgName;
pub use crate::engine::CommandName;
pub use crate::engine::SMCommandName;
