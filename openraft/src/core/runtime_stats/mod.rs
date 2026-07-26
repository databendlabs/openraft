//! Runtime statistics collection.
//!
//! Everything here is reachable only through `Raft::runtime_stats`, which exists only under the
//! `runtime-stats` feature. With the feature off the whole module is legitimately unreachable, so
//! the allow below is conditioned on that rather than applied outright: with the feature on, dead
//! code here is real dead code and still warns.
#![cfg_attr(not(feature = "runtime-stats"), allow(dead_code))]

mod display_mode;
#[allow(clippy::module_inception)]
mod runtime_stats;
mod runtime_stats_display;

pub mod log_stage;

pub use self::display_mode::DisplayMode;
pub use self::runtime_stats::RuntimeStats;
pub use self::runtime_stats_display::RuntimeStatsDisplay;
