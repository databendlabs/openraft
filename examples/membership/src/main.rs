//! Run one membership scenario, e.g.:
//!
//! ```shell
//! cargo run --manifest-path examples/membership/Cargo.toml -- --policy weighted
//! ```

use clap::Parser;
use membership::Policy;
use tracing_subscriber::EnvFilter;

/// Run an in-process OpenRaft cluster with one membership.
#[derive(Debug, Parser)]
#[command(about)]
struct Args {
    /// The membership and edit types to run.
    #[arg(long, value_enum)]
    policy: Policy,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Print the phases of this example by default; `RUST_LOG` replaces the filter.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("membership=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(false).without_time().init();

    let args = Args::parse();
    membership::run(args.policy).await?;

    tracing::info!("--- {}: every check passed", args.policy);
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn accepts_every_policy_name() {
        let names = ["majority", "threshold", "weighted", "hierarchical", "grid", "explicit"];
        for name in names {
            let args = Args::try_parse_from(["membership", "--policy", name]).unwrap();
            assert_eq!(name, args.policy.name());
        }
    }

    #[test]
    fn rejects_an_unknown_policy_name() {
        let res = Args::try_parse_from(["membership", "--policy", "quorum"]);
        assert!(res.is_err());
    }
}
