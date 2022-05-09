#[allow(clippy::module_inception)] mod config;
mod error;

#[cfg(test)] mod config_test;

pub use config::Config;
pub use config::SnapshotPolicy;
pub use error::ConfigError;
