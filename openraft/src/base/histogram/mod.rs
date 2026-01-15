#[allow(clippy::module_inception)]
mod histogram;
mod log_scale;
mod log_scale_config;
mod percentile_stats;
mod slot;

pub use histogram::Histogram;
#[allow(unused_imports)]
pub use log_scale::LOG_SCALE;
#[allow(unused_imports)]
pub use log_scale::LogScale;
#[allow(unused_imports)]
pub use log_scale::LogScale3;
#[allow(unused_imports)]
pub use log_scale_config::DefaultLogScaleConfig;
#[allow(unused_imports)]
pub use log_scale_config::LogScaleConfig;
pub use percentile_stats::PercentileStats;
