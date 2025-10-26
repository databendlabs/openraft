#[allow(clippy::module_inception)]
mod histogram;
mod percentile_stats;

pub(crate) use histogram::Histogram;
pub(crate) use percentile_stats::PercentileStats;
