//! Clap-based parsing for [`Config`].
//!
//! Gated behind `feature = "clap"` in [`super`]; functions and the
//! [`Config::build`] method here are only compiled when that feature is on.

use std::str::FromStr;

use anyerror::AnyError;
use clap::Parser;

use crate::Config;
use crate::SnapshotPolicy;
use crate::config::error::ConfigError;

/// Parse number with unit such as 5.3 KB
pub(super) fn parse_bytes_with_unit(src: &str) -> Result<u64, ConfigError> {
    let res = byte_unit::Byte::from_str(src).map_err(|e| ConfigError::InvalidNumber {
        invalid: src.to_string(),
        reason: e.to_string(),
    })?;

    Ok(res.as_u64())
}

pub(super) fn parse_snapshot_policy(src: &str) -> Result<SnapshotPolicy, ConfigError> {
    if src == "never" {
        return Ok(SnapshotPolicy::Never);
    }

    let elts = src.split(':').collect::<Vec<_>>();
    if elts.len() != 2 {
        return Err(ConfigError::InvalidSnapshotPolicy {
            syntax: "never|since_last:<num>".to_string(),
            invalid: src.to_string(),
        });
    }

    if elts[0] != "since_last" {
        return Err(ConfigError::InvalidSnapshotPolicy {
            syntax: "never|since_last:<num>".to_string(),
            invalid: src.to_string(),
        });
    }

    let n_logs = elts[1].parse::<u64>().map_err(|e| ConfigError::InvalidNumber {
        invalid: src.to_string(),
        reason: e.to_string(),
    })?;
    Ok(SnapshotPolicy::LogsSinceLast(n_logs))
}

impl Config {
    /// Build a `Config` instance from a series of command line arguments.
    ///
    /// The first element in `args` must be the application name.
    ///
    /// Only available when the `clap` feature is enabled (default).
    ///
    /// # Examples
    ///
    /// ```
    /// use openraft::Config;
    ///
    /// let config = Config::build(&[
    ///     "myapp",
    ///     "--election-timeout-min", "300",
    ///     "--election-timeout-max", "500",
    /// ])?;
    /// # Ok::<(), openraft::ConfigError>(())
    /// ```
    pub fn build(args: &[&str]) -> Result<Config, ConfigError> {
        let config = <Self as Parser>::try_parse_from(args).map_err(|e| ConfigError::ParseError {
            source: AnyError::from(&e),
            args: args.iter().map(|x| x.to_string()).collect(),
        })?;
        config.validate()
    }
}
