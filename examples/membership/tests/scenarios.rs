//! Runs each scenario through [`membership::run()`], the function that the CLI calls.

use membership::Policy;
use tracing_subscriber::EnvFilter;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn majority() -> anyhow::Result<()> {
    init_logging();
    membership::run(Policy::Majority).await
}

/// Writes the logs that `RUST_LOG` selects to the output of a failed test.
fn init_logging() {
    let subscriber = tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).with_test_writer();
    // Another test of this binary may have installed the subscriber already.
    let _ = subscriber.try_init();
}
