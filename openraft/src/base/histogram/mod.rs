#[allow(clippy::module_inception)]
mod histogram;
mod percentile_stats;

pub use histogram::Histogram;
pub use percentile_stats::PercentileStats;
