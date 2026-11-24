use std::sync::Arc;

use clap::Parser;
use openraft::Config;

/// Parse openraft `Config` flags, such as `--heartbeat-interval 100`, into a validated `Config`.
///
/// The driver passes its flags after `--` to every node, and parses them itself first, so that a
/// bad flag stops a run before any node starts.
pub(crate) fn parse(raft_args: &[String]) -> anyhow::Result<Arc<Config>> {
    let args = std::iter::once("openraft").chain(raft_args.iter().map(String::as_str));
    let config = Config::try_parse_from(args)?;
    Ok(Arc::new(config.validate()?))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn test_no_flags_give_the_default_config() {
        let config = parse(&[]).unwrap();

        assert_eq!(50, config.heartbeat_interval);
        assert_eq!(300, config.election_timeout_max);
    }

    #[test]
    fn test_flags_set_config_fields() {
        let args = ["--heartbeat-interval", "100", "--election-timeout-max", "500"].map(String::from);

        let config = parse(&args).unwrap();

        assert_eq!(100, config.heartbeat_interval);
        assert_eq!(500, config.election_timeout_max);
    }

    #[test]
    fn test_an_unknown_flag_is_an_error() {
        let args = ["--no-such-flag".to_string()];

        assert!(parse(&args).is_err());
    }
}
